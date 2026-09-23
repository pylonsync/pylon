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
}

struct Inner {
    map: HashMap<Key, Entry>,
    tick: u64,
}

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

    /// Publish a freshly hydrated doc unless another caller already did, and
    /// return whichever is cached. Two concurrent first reads may both
    /// hydrate; the loser's copy is dropped.
    pub fn get_or_insert(&self, entity: &str, row_id: &str, doc: DocHandle) -> DocHandle {
        let mut inner = self.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        let entry = inner.map.entry(key(entity, row_id)).or_insert(Entry {
            doc,
            last_used: tick,
        });
        entry.last_used = tick;
        let out = Arc::clone(&entry.doc);
        self.trim(&mut inner);
        out
    }

    /// Replace a row's cached doc.
    pub fn insert(&self, entity: &str, row_id: &str, doc: DocHandle) {
        let mut inner = self.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        inner.map.insert(
            key(entity, row_id),
            Entry {
                doc,
                last_used: tick,
            },
        );
        self.trim(&mut inner);
    }

    pub fn remove(&self, entity: &str, row_id: &str) {
        self.inner.lock().unwrap().map.remove(&key(entity, row_id));
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().map.clear();
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
        let first = cache.get_or_insert("E", "1", doc());
        let second = cache.get_or_insert("E", "1", doc());
        assert!(Arc::ptr_eq(&first, &second));
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
