//! Trait abstraction over `&mut postgres::Client` and
//! `&mut postgres::Transaction` so the maintenance + read helpers
//! (FTS, CRDT, etc.) work identically whether the caller holds a
//! standalone connection or an in-flight transaction.
//!
//! Without this trait every helper would need duplicate `_client` /
//! `_tx` variants; with it the Runtime can dispatch CRDT projection
//! + entity write + FTS maintenance through the *same* held
//! transaction, which is what guarantees atomicity across all three.

#![cfg(feature = "postgres-live")]

use postgres::types::ToSql;

/// Read + write operations available on either `Client` or
/// `Transaction`. The streaming callback avoids exposing a
/// connection-specific iterator lifetime.
pub trait PgConn {
    fn execute(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error>;

    fn query(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error>;

    /// Visit query rows. Built-in clients and transactions stream the result.
    /// The default buffers rows to support custom adapters that only implement `query`.
    fn query_each(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        visit: &mut dyn FnMut(postgres::Row),
    ) -> Result<(), postgres::Error> {
        for row in self.query(sql, params)? {
            visit(row);
        }
        Ok(())
    }

    fn query_opt(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error>;
}

impl PgConn for postgres::Client {
    fn execute(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error> {
        postgres::Client::execute(self, sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error> {
        postgres::Client::query(self, sql, params)
    }
    fn query_each(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        visit: &mut dyn FnMut(postgres::Row),
    ) -> Result<(), postgres::Error> {
        use postgres::fallible_iterator::FallibleIterator;
        let mut rows = postgres::Client::query_raw(self, sql, params.iter().copied())?;
        while let Some(row) = rows.next()? {
            visit(row);
        }
        Ok(())
    }
    fn query_opt(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error> {
        postgres::Client::query_opt(self, sql, params)
    }
}

impl<'a> PgConn for postgres::Transaction<'a> {
    fn execute(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error> {
        postgres::Transaction::execute(self, sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error> {
        postgres::Transaction::query(self, sql, params)
    }
    fn query_each(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        visit: &mut dyn FnMut(postgres::Row),
    ) -> Result<(), postgres::Error> {
        use postgres::fallible_iterator::FallibleIterator;
        let mut rows = postgres::Transaction::query_raw(self, sql, params.iter().copied())?;
        while let Some(row) = rows.next()? {
            visit(row);
        }
        Ok(())
    }
    fn query_opt(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error> {
        postgres::Transaction::query_opt(self, sql, params)
    }
}

/// Distinct statements one [`CachedTransaction`] keeps prepared. Past this
/// a statement runs unprepared (two round trips), as without the cache.
const MAX_CACHED_STATEMENTS: usize = 128;

/// A Postgres transaction that prepares each distinct SQL text once.
///
/// `postgres::Transaction::execute(&str, ..)` sends Parse and waits for the
/// parameter types, then sends Bind and Execute and waits again: two round
/// trips per statement. A mutation that writes many rows runs the same few
/// statements again and again, so this keeps each prepared statement until
/// the transaction ends and runs a repeat in one round trip.
///
/// The statements are closed when the transaction ends, so none outlives
/// it on the server connection: a schema change after the transaction
/// cannot make one stale, and a transaction-mode pooler can give the
/// connection to another client.
pub struct CachedTransaction<'a> {
    // Declared before `tx` so the statements drop first: their Close
    // messages go out with the transaction's ROLLBACK, not after it.
    statements: std::collections::HashMap<String, postgres::Statement>,
    tx: postgres::Transaction<'a>,
}

impl<'a> CachedTransaction<'a> {
    pub fn new(tx: postgres::Transaction<'a>) -> Self {
        Self {
            statements: std::collections::HashMap::new(),
            tx,
        }
    }

    /// Commit. The prepared statements are closed in the same flush.
    pub fn commit(self) -> Result<(), postgres::Error> {
        let Self { statements, tx } = self;
        drop(statements);
        tx.commit()
    }

    /// The prepared statement for `sql`, prepared now when it is new. None
    /// when the cache is full.
    fn statement(&mut self, sql: &str) -> Result<Option<postgres::Statement>, postgres::Error> {
        if let Some(statement) = self.statements.get(sql) {
            return Ok(Some(statement.clone()));
        }
        if self.statements.len() >= MAX_CACHED_STATEMENTS {
            return Ok(None);
        }
        let statement = self.tx.prepare(sql)?;
        self.statements.insert(sql.to_owned(), statement.clone());
        Ok(Some(statement))
    }

    pub fn execute(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error> {
        match self.statement(sql)? {
            Some(statement) => self.tx.execute(&statement, params),
            None => self.tx.execute(sql, params),
        }
    }

    pub fn query(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error> {
        match self.statement(sql)? {
            Some(statement) => self.tx.query(&statement, params),
            None => self.tx.query(sql, params),
        }
    }

    pub fn query_opt(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error> {
        match self.statement(sql)? {
            Some(statement) => self.tx.query_opt(&statement, params),
            None => self.tx.query_opt(sql, params),
        }
    }

    pub fn query_one(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<postgres::Row, postgres::Error> {
        match self.statement(sql)? {
            Some(statement) => self.tx.query_one(&statement, params),
            None => self.tx.query_one(sql, params),
        }
    }
}

impl<'a> std::ops::Deref for CachedTransaction<'a> {
    type Target = postgres::Transaction<'a>;
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl std::ops::DerefMut for CachedTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

impl PgConn for CachedTransaction<'_> {
    fn execute(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error> {
        CachedTransaction::execute(self, sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error> {
        CachedTransaction::query(self, sql, params)
    }
    fn query_each(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        visit: &mut dyn FnMut(postgres::Row),
    ) -> Result<(), postgres::Error> {
        use postgres::fallible_iterator::FallibleIterator;
        let mut rows = match self.statement(sql)? {
            Some(statement) => self.tx.query_raw(&statement, params.iter().copied())?,
            None => self.tx.query_raw(sql, params.iter().copied())?,
        };
        while let Some(row) = rows.next()? {
            visit(row);
        }
        Ok(())
    }
    fn query_opt(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error> {
        CachedTransaction::query_opt(self, sql, params)
    }
}
