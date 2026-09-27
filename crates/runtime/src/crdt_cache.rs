//! Bounded in-memory cache of per-row Loro documents, shared by the SQLite
//! (`loro_store`) and Postgres (`pg_loro_store`) CRDT stores.
//!
//! Both stores used an unbounded `HashMap` and never evicted on delete, so
//! every row a process wrote stayed in memory until it restarted. An app
//! whose crons replace thousands of rollup rows an hour (Stack0 Analytics,
//! 2026-09) grew until the machine was OOM-killed. The cache is only an
//! accelerator: a miss re-hydrates from the `_pylon_crdt_snapshots` sidecar,
//! so dropping entries never loses data.
//!
//! When the cache passes its capacity it drops the least recently used tenth
//! in one pass, so the O(n) scan runs once per `capacity / 10` inserts rather
//! than on every insert.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pylon_crdt::loro::LoroDoc;

/// Rows kept when `PYLON_CRDT_CACHE_ROWS` is unset.
pub const DEFAULT_CAPACITY: usize = 10_000;

type Key = (String, String);
pub type DocHandle = Arc<Mutex<LoroDoc>>;

struct Entry {
    doc: DocHandle,
    last_used: u64,
    /// The doc holds every field its row has a value for (see
    /// [`DocCache::mark_complete`]).
    complete: bool,
    /// The stored snapshot's version the doc was read from (Postgres:
    /// its `updated_at`), so a read can tell another process wrote it.
    stamp: Option<i64>,
}

struct Inner {
    map: HashMap<Key, Entry>,
    tick: u64,
    /// Per row, raised by every insert and remove: a read that hydrated
    /// from storage caches its doc only if no write replaced or evicted the
    /// row's doc since it started ([`DocCache::token`]).
    generations: HashMap<Key, u64>,
    /// Raised when `generations` is cleared to bound it.
    epoch: u64,
}

/// A row's cache state when a read started; see [`DocCache::token`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token(u64, u64);

pub struct DocCache {
    inner: Mutex<Inner>,
    capacity: usize,
}

impl Default for DocCache {
    fn default() -> Self {
        Self::with_capacity(capacity_from_env())
    }
}

/// `PYLON_CRDT_CACHE_ROWS`, or [`DEFAULT_CAPACITY`]. Zero or garbage falls
/// back to the default; a cache of zero rows would re-decode on every read.
pub fn capacity_from_env() -> usize {
    std::env::var("PYLON_CRDT_CACHE_ROWS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_CAPACITY)
}

fn key(entity: &str, row_id: &str) -> Key {
    (entity.to_string(), row_id.to_string())
}

impl DocCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                tick: 0,
                generations: HashMap::new(),
                epoch: 0,
            }),
            capacity: capacity.max(1),
        }
    }

    /// The cached doc for a row, marking it recently used.
    pub fn get(&self, entity: &str, row_id: &str) -> Option<DocHandle> {
        let mut inner = self.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        inner.map.get_mut(&key(entity, row_id)).map(|e| {
            e.last_used = tick;
            Arc::clone(&e.doc)
        })
    }

    /// The row's cache state now. Take it before reading the row's doc
    /// from storage and pass it to [`DocCache::get_or_insert`]: a write that
    /// committed and evicted the row in between makes the read's doc stale.
    pub fn token(&self, entity: &str, row_id: &str) -> Token {
        let inner = self.inner.lock().unwrap();
        let generation = inner.generations.get(&key(entity, row_id)).copied();
        Token(inner.epoch, generation.unwrap_or(0))
    }

    /// Publish a freshly hydrated doc unless another caller already did, and
    /// return whichever is cached. Two concurrent first reads may both
    /// hydrate; the loser's copy is dropped. When the row was inserted or
    /// removed since `token` was taken, the doc is returned uncached: it may
    /// predate that write.
    pub fn get_or_insert(
        &self,
        entity: &str,
        row_id: &str,
        doc: DocHandle,
        token: Token,
    ) -> DocHandle {
        let mut inner = self.inner.lock().unwrap();
        let k = key(entity, row_id);
        let now = Token(inner.epoch, inner.generations.get(&k).copied().unwrap_or(0));
        if now != token {
            return inner.map.get(&k).map(|e| Arc::clone(&e.doc)).unwrap_or(doc);
        }
        inner.tick += 1;
        let tick = inner.tick;
        let entry = inner.map.entry(k).or_insert(Entry {
            doc,
            last_used: tick,
            complete: false,
            stamp: None,
        });
        entry.last_used = tick;
        let out = Arc::clone(&entry.doc);
        self.trim(&mut inner);
        out
    }

    /// The cached doc for a row and the stored version it was read from,
    /// marking it recently used.
    pub fn get_stamped(&self, entity: &str, row_id: &str) -> Option<(DocHandle, Option<i64>)> {
        let mut inner = self.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        inner.map.get_mut(&key(entity, row_id)).map(|e| {
            e.last_used = tick;
            (Arc::clone(&e.doc), e.stamp)
        })
    }

    /// Cache a doc read from storage at version `stamp`, replacing a cached
    /// doc of another version, and return the cached one. When the row was
    /// inserted or removed since `token` was taken, the doc is returned
    /// uncached: it may predate that write.
    pub fn publish(
        &self,
        entity: &str,
        row_id: &str,
        doc: DocHandle,
        stamp: Option<i64>,
        token: Token,
    ) -> DocHandle {
        let mut inner = self.inner.lock().unwrap();
        let k = key(entity, row_id);
        let now = Token(inner.epoch, inner.generations.get(&k).copied().unwrap_or(0));
        if now != token {
            return doc;
        }
        inner.tick += 1;
        let tick = inner.tick;
        if let Some(e) = inner.map.get_mut(&k) {
            if e.stamp == stamp {
                e.last_used = tick;
                return Arc::clone(&e.doc);
            }
        }
        self.bump(&mut inner, &k);
        inner.map.insert(
            k,
            Entry {
                doc: Arc::clone(&doc),
                last_used: tick,
                complete: false,
                stamp,
            },
        );
        self.trim(&mut inner);
        doc
    }

    /// Whether the row's cached doc is marked complete.
    pub fn is_complete(&self, entity: &str, row_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .map
            .get(&key(entity, row_id))
            .is_some_and(|e| e.complete)
    }

    /// Mark the row's cached doc as holding every field its row has a value
    /// for, when nothing inserted or removed it since `token`: reads then
    /// skip the row. Cleared when the doc leaves the cache.
    pub fn mark_complete(&self, entity: &str, row_id: &str, token: Token) {
        let mut inner = self.inner.lock().unwrap();
        let k = key(entity, row_id);
        let now = Token(inner.epoch, inner.generations.get(&k).copied().unwrap_or(0));
        if now == token {
            if let Some(e) = inner.map.get_mut(&k) {
                e.complete = true;
            }
        }
    }

    fn bump(&self, inner: &mut Inner, k: &Key) {
        if inner.generations.len() >= self.capacity.saturating_mul(4) {
            inner.generations.clear();
            inner.epoch += 1;
        }
        *inner.generations.entry(k.clone()).or_insert(0) += 1;
    }

    /// Replace a row's cached doc.
    pub fn insert(&self, entity: &str, row_id: &str, doc: DocHandle) {
        let mut inner = self.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        let k = key(entity, row_id);
        self.bump(&mut inner, &k);
        inner.map.insert(
            k,
            Entry {
                doc,
                last_used: tick,
                complete: false,
                stamp: None,
            },
        );
        self.trim(&mut inner);
    }

    pub fn remove(&self, entity: &str, row_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        let k = key(entity, row_id);
        self.bump(&mut inner, &k);
        inner.map.remove(&k);
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.map.clear();
        inner.generations.clear();
        inner.epoch += 1;
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn trim(&self, inner: &mut Inner) {
        if inner.map.len() <= self.capacity {
            return;
        }
        // Down to 90% of capacity, oldest first.
        let target = self.capacity - self.capacity / 10;
        let excess = inner.map.len() - target;
        let mut by_age: Vec<(u64, Key)> = inner
            .map
            .iter()
            .map(|(k, e)| (e.last_used, k.clone()))
            .collect();
        by_age.select_nth_unstable_by_key(excess - 1, |(t, _)| *t);
        for (_, k) in by_age.into_iter().take(excess) {
            inner.map.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> DocHandle {
        Arc::new(Mutex::new(LoroDoc::new()))
    }

    #[test]
    fn stays_within_capacity() {
        let cache = DocCache::with_capacity(100);
        for i in 0..1_000 {
            cache.insert("E", &i.to_string(), doc());
            assert!(cache.len() <= 100, "len {} at insert {i}", cache.len());
        }
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let cache = DocCache::with_capacity(10);
        for i in 0..10 {
            cache.insert("E", &i.to_string(), doc());
        }
        // Touch 0..5 so 5..10 become the oldest.
        for i in 0..5 {
            assert!(cache.get("E", &i.to_string()).is_some());
        }
        cache.insert("E", "new", doc()); // 11 > 10: trims to 9.
        for i in 0..5 {
            assert!(
                cache.get("E", &i.to_string()).is_some(),
                "recent row {i} evicted"
            );
        }
        assert!(cache.get("E", "new").is_some());
        assert!(cache.get("E", "5").is_none(), "oldest row kept");
    }

    #[test]
    fn get_or_insert_keeps_the_first_doc() {
        let cache = DocCache::with_capacity(10);
        let first = cache.get_or_insert("E", "1", doc(), cache.token("E", "1"));
        let second = cache.get_or_insert("E", "1", doc(), cache.token("E", "1"));
        assert!(Arc::ptr_eq(&first, &second));
    }

    /// A read that hydrated before a write evicted the row does not cache
    /// its doc (it may predate the write); a read after it does.
    #[test]
    fn a_read_from_before_an_eviction_is_not_cached() {
        let cache = DocCache::with_capacity(10);
        let before = cache.token("E", "1");
        cache.remove("E", "1"); // A write committed and evicted the row.
        let stale = doc();
        let got = cache.get_or_insert("E", "1", Arc::clone(&stale), before);
        assert!(Arc::ptr_eq(&got, &stale));
        assert!(cache.get("E", "1").is_none(), "the stale doc was cached");
        let fresh = cache.get_or_insert("E", "1", doc(), cache.token("E", "1"));
        assert!(Arc::ptr_eq(&cache.get("E", "1").unwrap(), &fresh));
        // A clear (new peers, a restored database) invalidates tokens too.
        let before = cache.token("E", "2");
        cache.clear();
        cache.get_or_insert("E", "2", doc(), before);
        assert!(cache.get("E", "2").is_none());
    }

    /// The complete mark holds until the doc leaves the cache, and is not
    /// set from a read that a write overtook.
    #[test]
    fn the_complete_mark_ends_with_the_cached_doc() {
        let cache = DocCache::with_capacity(10);
        let token = cache.token("E", "1");
        cache.get_or_insert("E", "1", doc(), token);
        cache.mark_complete("E", "1", token);
        assert!(cache.is_complete("E", "1"));
        cache.remove("E", "1");
        assert!(!cache.is_complete("E", "1"));
        let token = cache.token("E", "1");
        cache.get_or_insert("E", "1", doc(), token);
        cache.insert("E", "1", doc()); // A write replaced the doc.
        cache.mark_complete("E", "1", token);
        assert!(!cache.is_complete("E", "1"));
    }

    /// A doc read at another stored version (another process wrote the
    /// row) replaces the cached one and its complete mark; the same version
    /// keeps it.
    #[test]
    fn a_new_stored_version_replaces_the_cached_doc() {
        let cache = DocCache::with_capacity(10);
        let first = cache.publish("E", "1", doc(), Some(1), cache.token("E", "1"));
        let token = cache.token("E", "1");
        cache.mark_complete("E", "1", token);
        let same = cache.publish("E", "1", doc(), Some(1), cache.token("E", "1"));
        assert!(Arc::ptr_eq(&first, &same));
        assert!(cache.is_complete("E", "1"));
        let newer = cache.publish("E", "1", doc(), Some(2), cache.token("E", "1"));
        assert!(!Arc::ptr_eq(&first, &newer));
        assert_eq!(cache.get_stamped("E", "1").unwrap().1, Some(2));
        assert!(!cache.is_complete("E", "1"));
    }

    #[test]
    fn remove_drops_the_row() {
        let cache = DocCache::with_capacity(10);
        cache.insert("E", "1", doc());
        cache.remove("E", "1");
        assert!(cache.get("E", "1").is_none());
        assert!(cache.is_empty());
    }
}
