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
