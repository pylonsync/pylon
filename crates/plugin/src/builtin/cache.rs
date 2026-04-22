use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::Plugin;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// The value types the cache can store.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum CacheValue {
    String(String),
    Int(i64),
    Float(f64),
    List(VecDeque<String>),
    Set(HashSet<String>),
    Hash(HashMap<String, String>),
    SortedSet(BTreeMap<String, f64>),
}

/// An entry in the cache with optional expiration.
struct CacheEntry {
    value: CacheValue,
    expires_at: Option<Instant>,
    #[allow(dead_code)]
    created_at: Instant,
    last_accessed: Instant,
}

impl CacheEntry {
    fn new(value: CacheValue, ttl: Option<u64>) -> Self {
        let now = Instant::now();
        Self {
            value,
            expires_at: ttl.map(|s| now + Duration::from_secs(s)),
            created_at: now,
            last_accessed: now,
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Instant::now() >= exp)
            .unwrap_or(false)
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// The cache engine -- a Redis-like in-memory data structure store.
pub struct CachePlugin {
    store: Mutex<HashMap<String, CacheEntry>>,
    max_keys: usize,
    stats: Mutex<CacheStats>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub sets: u64,
    pub deletes: u64,
    pub evictions: u64,
    pub expired: u64,
}

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

fn glob_match(pattern: &str, text: &str) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let pb = pattern.as_bytes();
    let tb = text.as_bytes();
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;

    while ti < tb.len() {
        if pi < pb.len() && (pb[pi] == b'?' || pb[pi] == tb[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pb.len() && pb[pi] == b'*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pb.len() && pb[pi] == b'*' {
        pi += 1;
    }
    pi == pb.len()
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl CachePlugin {
    pub fn new(max_keys: usize) -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
            max_keys,
            stats: Mutex::new(CacheStats::default()),
        }
    }

    // -- internal helpers ---------------------------------------------------

    /// Record a cache hit.
    fn record_hit(&self) {
        self.stats.lock().unwrap().hits += 1;
    }

    /// Record a cache miss.
    fn record_miss(&self) {
        self.stats.lock().unwrap().misses += 1;
    }

    /// Evict the least-recently-used key to make room for a new entry.
    /// Caller must already hold the store lock.
    fn evict_lru(&self, store: &mut HashMap<String, CacheEntry>) {
        if store.len() < self.max_keys {
            return;
        }

        // Find the key with the oldest last_accessed timestamp.
        let victim = store
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(k, _)| k.clone());

        if let Some(key) = victim {
            store.remove(&key);
            self.stats.lock().unwrap().evictions += 1;
        }
    }

    /// Remove a key if it is expired. Returns true if the key was removed.
    /// Caller must already hold the store lock.
    fn remove_if_expired(
        &self,
        store: &mut HashMap<String, CacheEntry>,
        key: &str,
    ) -> bool {
        let expired = store.get(key).map(|e| e.is_expired()).unwrap_or(false);
        if expired {
            store.remove(key);
            self.stats.lock().unwrap().expired += 1;
        }
        expired
    }

    // -----------------------------------------------------------------------
    // String operations
    // -----------------------------------------------------------------------

    /// SET key value [EX seconds]
    pub fn set(&self, key: &str, value: &str, ttl: Option<u64>) {
        let mut store = self.store.lock().unwrap();
        self.evict_lru(&mut store);
        store.insert(
            key.to_string(),
            CacheEntry::new(CacheValue::String(value.to_string()), ttl),
        );
        self.stats.lock().unwrap().sets += 1;
    }

    /// GET key -- returns None if expired or missing.
    pub fn get(&self, key: &str) -> Option<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            self.record_miss();
            return None;
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                let val = match &entry.value {
                    CacheValue::String(s) => Some(s.clone()),
                    CacheValue::Int(n) => Some(n.to_string()),
                    CacheValue::Float(f) => Some(f.to_string()),
                    _ => None,
                };
                if val.is_some() {
                    self.record_hit();
                } else {
                    self.record_miss();
                }
                val
            }
            None => {
                self.record_miss();
                None
            }
        }
    }

    /// DEL key -- returns true if key existed.
    pub fn del(&self, key: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        let existed = store.remove(key).is_some();
        if existed {
            self.stats.lock().unwrap().deletes += 1;
        }
        existed
    }

    /// EXISTS key
    pub fn exists(&self, key: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        store.contains_key(key)
    }

    /// INCR key -- increment integer value, creates if not exists (starts at 0).
    pub fn incr(&self, key: &str) -> Result<i64, String> {
        self.incrby(key, 1)
    }

    /// DECR key
    pub fn decr(&self, key: &str) -> Result<i64, String> {
        self.incrby(key, -1)
    }

    /// INCRBY key amount
    pub fn incrby(&self, key: &str, amount: i64) -> Result<i64, String> {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                match &mut entry.value {
                    CacheValue::Int(n) => {
                        *n += amount;
                        Ok(*n)
                    }
                    CacheValue::String(s) => {
                        let n: i64 = s
                            .parse()
                            .map_err(|_| "value is not an integer".to_string())?;
                        let new_val = n + amount;
                        entry.value = CacheValue::Int(new_val);
                        Ok(new_val)
                    }
                    _ => Err("value is not an integer".to_string()),
                }
            }
            None => {
                self.evict_lru(&mut store);
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::Int(amount), None),
                );
                Ok(amount)
            }
        }
    }

    /// SETNX -- set only if key doesn't exist. Returns true if set.
    pub fn setnx(&self, key: &str, value: &str, ttl: Option<u64>) -> bool {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        if store.contains_key(key) {
            return false;
        }

        self.evict_lru(&mut store);
        store.insert(
            key.to_string(),
            CacheEntry::new(CacheValue::String(value.to_string()), ttl),
        );
        self.stats.lock().unwrap().sets += 1;
        true
    }

    /// GETSET -- set new value and return old value.
    pub fn getset(&self, key: &str, value: &str) -> Option<String> {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        let old = store.get(key).and_then(|entry| match &entry.value {
            CacheValue::String(s) => Some(s.clone()),
            CacheValue::Int(n) => Some(n.to_string()),
            CacheValue::Float(f) => Some(f.to_string()),
            _ => None,
        });

        self.evict_lru(&mut store);
        store.insert(
            key.to_string(),
            CacheEntry::new(CacheValue::String(value.to_string()), None),
        );
        self.stats.lock().unwrap().sets += 1;
        old
    }

    /// MGET -- get multiple keys.
    pub fn mget(&self, keys: &[&str]) -> Vec<Option<String>> {
        let mut store = self.store.lock().unwrap();
        keys.iter()
            .map(|key| {
                if self.remove_if_expired(&mut store, key) {
                    self.record_miss();
                    return None;
                }
                match store.get_mut(*key) {
                    Some(entry) => {
                        entry.touch();
                        match &entry.value {
                            CacheValue::String(s) => {
                                self.record_hit();
                                Some(s.clone())
                            }
                            CacheValue::Int(n) => {
                                self.record_hit();
                                Some(n.to_string())
                            }
                            CacheValue::Float(f) => {
                                self.record_hit();
                                Some(f.to_string())
                            }
                            _ => {
                                self.record_miss();
                                None
                            }
                        }
                    }
                    None => {
                        self.record_miss();
                        None
                    }
                }
            })
            .collect()
    }

    /// MSET -- set multiple keys.
    pub fn mset(&self, pairs: &[(&str, &str)]) {
        let mut store = self.store.lock().unwrap();
        for (key, value) in pairs {
            self.evict_lru(&mut store);
            store.insert(
                key.to_string(),
                CacheEntry::new(CacheValue::String(value.to_string()), None),
            );
            self.stats.lock().unwrap().sets += 1;
        }
    }

    /// TTL key -- returns remaining seconds, -1 if no expiry, -2 if key doesn't exist.
    pub fn ttl(&self, key: &str) -> i64 {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return -2;
        }
        match store.get(key) {
            Some(entry) => match entry.expires_at {
                Some(exp) => {
                    let now = Instant::now();
                    if exp > now {
                        (exp - now).as_secs() as i64
                    } else {
                        -2
                    }
                }
                None => -1,
            },
            None => -2,
        }
    }

    /// EXPIRE key seconds -- set expiry on existing key.
    pub fn expire(&self, key: &str, seconds: u64) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.expires_at = Some(Instant::now() + Duration::from_secs(seconds));
                true
            }
            None => false,
        }
    }

    /// PERSIST key -- remove expiry.
    pub fn persist(&self, key: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get_mut(key) {
            Some(entry) => {
                let had_expiry = entry.expires_at.is_some();
                entry.expires_at = None;
                had_expiry
            }
            None => false,
        }
    }

    /// KEYS pattern -- find keys matching glob pattern.
    pub fn keys(&self, pattern: &str) -> Vec<String> {
        let mut store = self.store.lock().unwrap();

        // First collect expired keys so we can remove them.
        let expired: Vec<String> = store
            .iter()
            .filter(|(_, entry)| entry.is_expired())
            .map(|(k, _)| k.clone())
            .collect();
        for k in &expired {
            store.remove(k);
        }
        {
            let mut stats = self.stats.lock().unwrap();
            stats.expired += expired.len() as u64;
        }

        store
            .keys()
            .filter(|k| glob_match(pattern, k))
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // List operations
    // -----------------------------------------------------------------------

    /// LPUSH key value -- push to front, creates list if needed.
    pub fn lpush(&self, key: &str, value: &str) -> usize {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::List(list) = &mut entry.value {
                    list.push_front(value.to_string());
                    list.len()
                } else {
                    // Replace with a new list containing the value.
                    let mut list = VecDeque::new();
                    list.push_front(value.to_string());
                    let len = list.len();
                    entry.value = CacheValue::List(list);
                    len
                }
            }
            None => {
                self.evict_lru(&mut store);
                let mut list = VecDeque::new();
                list.push_front(value.to_string());
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::List(list), None),
                );
                1
            }
        }
    }

    /// RPUSH key value -- push to back.
    pub fn rpush(&self, key: &str, value: &str) -> usize {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::List(list) = &mut entry.value {
                    list.push_back(value.to_string());
                    list.len()
                } else {
                    let mut list = VecDeque::new();
                    list.push_back(value.to_string());
                    let len = list.len();
                    entry.value = CacheValue::List(list);
                    len
                }
            }
            None => {
                self.evict_lru(&mut store);
                let mut list = VecDeque::new();
                list.push_back(value.to_string());
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::List(list), None),
                );
                1
            }
        }
    }

    /// LPOP key -- pop from front.
    pub fn lpop(&self, key: &str) -> Option<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return None;
        }
        let entry = store.get_mut(key)?;
        entry.touch();
        if let CacheValue::List(list) = &mut entry.value {
            list.pop_front()
        } else {
            None
        }
    }

    /// RPOP key -- pop from back.
    pub fn rpop(&self, key: &str) -> Option<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return None;
        }
        let entry = store.get_mut(key)?;
        entry.touch();
        if let CacheValue::List(list) = &mut entry.value {
            list.pop_back()
        } else {
            None
        }
    }

    /// LRANGE key start stop -- get range (inclusive, supports negative indices).
    pub fn lrange(&self, key: &str, start: i64, stop: i64) -> Vec<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return vec![];
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::List(list) = &entry.value {
                    let len = list.len() as i64;
                    if len == 0 {
                        return vec![];
                    }

                    // Resolve negative indices.
                    let s = if start < 0 {
                        (len + start).max(0) as usize
                    } else {
                        start.min(len - 1) as usize
                    };
                    let e = if stop < 0 {
                        (len + stop).max(0) as usize
                    } else {
                        stop.min(len - 1) as usize
                    };

                    if s > e {
                        return vec![];
                    }

                    list.iter().skip(s).take(e - s + 1).cloned().collect()
                } else {
                    vec![]
                }
            }
            None => vec![],
        }
    }

    /// LLEN key -- list length.
    pub fn llen(&self, key: &str) -> usize {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return 0;
        }
        match store.get(key) {
            Some(entry) => {
                if let CacheValue::List(list) = &entry.value {
                    list.len()
                } else {
                    0
                }
            }
            None => 0,
        }
    }

    // -----------------------------------------------------------------------
    // Set operations
    // -----------------------------------------------------------------------

    /// SADD key member -- add to set. Returns true if the member was newly added.
    pub fn sadd(&self, key: &str, member: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Set(set) = &mut entry.value {
                    set.insert(member.to_string())
                } else {
                    let mut set = HashSet::new();
                    set.insert(member.to_string());
                    entry.value = CacheValue::Set(set);
                    true
                }
            }
            None => {
                self.evict_lru(&mut store);
                let mut set = HashSet::new();
                set.insert(member.to_string());
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::Set(set), None),
                );
                true
            }
        }
    }

    /// SREM key member -- remove from set.
    pub fn srem(&self, key: &str, member: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Set(set) = &mut entry.value {
                    set.remove(member)
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// SMEMBERS key -- all members.
    pub fn smembers(&self, key: &str) -> Vec<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return vec![];
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Set(set) = &entry.value {
                    set.iter().cloned().collect()
                } else {
                    vec![]
                }
            }
            None => vec![],
        }
    }

    /// SISMEMBER key member
    pub fn sismember(&self, key: &str, member: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get(key) {
            Some(entry) => {
                if let CacheValue::Set(set) = &entry.value {
                    set.contains(member)
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// SCARD key -- set size.
    pub fn scard(&self, key: &str) -> usize {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return 0;
        }
        match store.get(key) {
            Some(entry) => {
                if let CacheValue::Set(set) = &entry.value {
                    set.len()
                } else {
                    0
                }
            }
            None => 0,
        }
    }

    /// SINTER key1 key2 -- intersection of two sets.
    pub fn sinter(&self, key1: &str, key2: &str) -> Vec<String> {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key1);
        self.remove_if_expired(&mut store, key2);

        let set1 = match store.get(key1) {
            Some(entry) => match &entry.value {
                CacheValue::Set(s) => s.clone(),
                _ => return vec![],
            },
            None => return vec![],
        };
        let set2 = match store.get(key2) {
            Some(entry) => match &entry.value {
                CacheValue::Set(s) => s,
                _ => return vec![],
            },
            None => return vec![],
        };

        set1.intersection(set2).cloned().collect()
    }

    /// SUNION key1 key2 -- union of two sets.
    pub fn sunion(&self, key1: &str, key2: &str) -> Vec<String> {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key1);
        self.remove_if_expired(&mut store, key2);

        let set1 = match store.get(key1) {
            Some(entry) => match &entry.value {
                CacheValue::Set(s) => s.clone(),
                _ => HashSet::new(),
            },
            None => HashSet::new(),
        };
        let set2 = match store.get(key2) {
            Some(entry) => match &entry.value {
                CacheValue::Set(s) => s,
                _ => return set1.into_iter().collect(),
            },
            None => return set1.into_iter().collect(),
        };

        set1.union(set2).cloned().collect()
    }

    // -----------------------------------------------------------------------
    // Hash operations
    // -----------------------------------------------------------------------

    /// HSET key field value
    pub fn hset(&self, key: &str, field: &str, value: &str) {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Hash(hash) = &mut entry.value {
                    hash.insert(field.to_string(), value.to_string());
                } else {
                    let mut hash = HashMap::new();
                    hash.insert(field.to_string(), value.to_string());
                    entry.value = CacheValue::Hash(hash);
                }
            }
            None => {
                self.evict_lru(&mut store);
                let mut hash = HashMap::new();
                hash.insert(field.to_string(), value.to_string());
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::Hash(hash), None),
                );
            }
        }
    }

    /// HGET key field
    pub fn hget(&self, key: &str, field: &str) -> Option<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return None;
        }
        let entry = store.get_mut(key)?;
        entry.touch();
        if let CacheValue::Hash(hash) = &entry.value {
            hash.get(field).cloned()
        } else {
            None
        }
    }

    /// HDEL key field
    pub fn hdel(&self, key: &str, field: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Hash(hash) = &mut entry.value {
                    hash.remove(field).is_some()
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// HGETALL key -- all field-value pairs.
    pub fn hgetall(&self, key: &str) -> HashMap<String, String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return HashMap::new();
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Hash(hash) = &entry.value {
                    hash.clone()
                } else {
                    HashMap::new()
                }
            }
            None => HashMap::new(),
        }
    }

    /// HEXISTS key field
    pub fn hexists(&self, key: &str, field: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get(key) {
            Some(entry) => {
                if let CacheValue::Hash(hash) = &entry.value {
                    hash.contains_key(field)
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// HLEN key
    pub fn hlen(&self, key: &str) -> usize {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return 0;
        }
        match store.get(key) {
            Some(entry) => {
                if let CacheValue::Hash(hash) = &entry.value {
                    hash.len()
                } else {
                    0
                }
            }
            None => 0,
        }
    }

    /// HKEYS key -- all field names.
    pub fn hkeys(&self, key: &str) -> Vec<String> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return vec![];
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Hash(hash) = &entry.value {
                    hash.keys().cloned().collect()
                } else {
                    vec![]
                }
            }
            None => vec![],
        }
    }

    /// HINCRBY key field amount
    pub fn hincrby(&self, key: &str, field: &str, amount: i64) -> Result<i64, String> {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::Hash(hash) = &mut entry.value {
                    let current: i64 = match hash.get(field) {
                        Some(v) => v
                            .parse()
                            .map_err(|_| "hash value is not an integer".to_string())?,
                        None => 0,
                    };
                    let new_val = current + amount;
                    hash.insert(field.to_string(), new_val.to_string());
                    Ok(new_val)
                } else {
                    Err("key is not a hash".to_string())
                }
            }
            None => {
                self.evict_lru(&mut store);
                let mut hash = HashMap::new();
                hash.insert(field.to_string(), amount.to_string());
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::Hash(hash), None),
                );
                Ok(amount)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Sorted set operations
    // -----------------------------------------------------------------------

    /// ZADD key score member
    pub fn zadd(&self, key: &str, score: f64, member: &str) {
        let mut store = self.store.lock().unwrap();
        self.remove_if_expired(&mut store, key);

        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::SortedSet(zset) = &mut entry.value {
                    zset.insert(member.to_string(), score);
                } else {
                    let mut zset = BTreeMap::new();
                    zset.insert(member.to_string(), score);
                    entry.value = CacheValue::SortedSet(zset);
                }
            }
            None => {
                self.evict_lru(&mut store);
                let mut zset = BTreeMap::new();
                zset.insert(member.to_string(), score);
                store.insert(
                    key.to_string(),
                    CacheEntry::new(CacheValue::SortedSet(zset), None),
                );
            }
        }
    }

    /// ZREM key member
    pub fn zrem(&self, key: &str, member: &str) -> bool {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return false;
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::SortedSet(zset) = &mut entry.value {
                    zset.remove(member).is_some()
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// ZSCORE key member
    pub fn zscore(&self, key: &str, member: &str) -> Option<f64> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return None;
        }
        let entry = store.get_mut(key)?;
        entry.touch();
        if let CacheValue::SortedSet(zset) = &entry.value {
            zset.get(member).copied()
        } else {
            None
        }
    }

    /// ZRANK key member -- rank by score (0-based).
    pub fn zrank(&self, key: &str, member: &str) -> Option<usize> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return None;
        }
        let entry = store.get_mut(key)?;
        entry.touch();
        if let CacheValue::SortedSet(zset) = &entry.value {
            let target_score = zset.get(member)?;
            // Sort by score, then by member name for deterministic ordering.
            let mut members: Vec<(&String, &f64)> = zset.iter().collect();
            members.sort_by(|a, b| {
                a.1.partial_cmp(b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            });
            members
                .iter()
                .position(|(m, s)| *m == member && *s == target_score)
        } else {
            None
        }
    }

    /// ZRANGE key start stop -- members by rank range (inclusive).
    pub fn zrange(&self, key: &str, start: usize, stop: usize) -> Vec<(String, f64)> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return vec![];
        }
        match store.get_mut(key) {
            Some(entry) => {
                entry.touch();
                if let CacheValue::SortedSet(zset) = &entry.value {
                    let mut members: Vec<(String, f64)> = zset
                        .iter()
                        .map(|(m, s)| (m.clone(), *s))
                        .collect();
                    members.sort_by(|a, b| {
                        a.1.partial_cmp(&b.1)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| a.0.cmp(&b.0))
                    });
                    let end = stop.min(members.len().saturating_sub(1));
                    if start > end {
                        return vec![];
                    }
                    members[start..=end].to_vec()
                } else {
                    vec![]
                }
            }
            None => vec![],
        }
    }

    /// ZCARD key -- sorted set size.
    pub fn zcard(&self, key: &str) -> usize {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return 0;
        }
        match store.get(key) {
            Some(entry) => {
                if let CacheValue::SortedSet(zset) = &entry.value {
                    zset.len()
                } else {
                    0
                }
            }
            None => 0,
        }
    }

    // -----------------------------------------------------------------------
    // Utility operations
    // -----------------------------------------------------------------------

    /// DBSIZE -- total key count (excluding expired).
    pub fn dbsize(&self) -> usize {
        let store = self.store.lock().unwrap();
        store.values().filter(|e| !e.is_expired()).count()
    }

    /// FLUSHALL -- delete everything.
    pub fn flushall(&self) {
        let mut store = self.store.lock().unwrap();
        store.clear();
        let mut stats = self.stats.lock().unwrap();
        *stats = CacheStats::default();
    }

    /// INFO -- cache statistics.
    pub fn info(&self) -> CacheStats {
        self.stats.lock().unwrap().clone()
    }

    /// TYPE key -- returns the type of value stored.
    pub fn key_type(&self, key: &str) -> Option<&'static str> {
        let mut store = self.store.lock().unwrap();
        if self.remove_if_expired(&mut store, key) {
            return None;
        }
        store.get(key).map(|entry| match &entry.value {
            CacheValue::String(_) => "string",
            CacheValue::Int(_) => "string",
            CacheValue::Float(_) => "string",
            CacheValue::List(_) => "list",
            CacheValue::Set(_) => "set",
            CacheValue::Hash(_) => "hash",
            CacheValue::SortedSet(_) => "zset",
        })
    }

    /// Cleanup expired keys (call periodically). Returns number of keys removed.
    pub fn cleanup_expired(&self) -> usize {
        let mut store = self.store.lock().unwrap();
        let expired: Vec<String> = store
            .iter()
            .filter(|(_, entry)| entry.is_expired())
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired.len();
        for k in &expired {
            store.remove(k);
        }
        self.stats.lock().unwrap().expired += count as u64;
        count
    }
}

// ---------------------------------------------------------------------------
// Plugin trait implementation
// ---------------------------------------------------------------------------

impl Plugin for CachePlugin {
    fn name(&self) -> &str {
        "cache"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn cache() -> CachePlugin {
        CachePlugin::new(1000)
    }

    // -- String operations --------------------------------------------------

    #[test]
    fn set_and_get() {
        let c = cache();
        c.set("hello", "world", None);
        assert_eq!(c.get("hello"), Some("world".to_string()));
    }

    #[test]
    fn get_missing_key_returns_none() {
        let c = cache();
        assert_eq!(c.get("nonexistent"), None);
    }

    #[test]
    fn set_with_ttl_and_get_before_expiry() {
        let c = cache();
        c.set("k", "v", Some(10));
        assert_eq!(c.get("k"), Some("v".to_string()));
    }

    #[test]
    fn get_expired_key_returns_none() {
        let c = cache();
        c.set("k", "v", Some(0));
        // TTL of 0 seconds means it expires immediately.
        thread::sleep(Duration::from_millis(5));
        assert_eq!(c.get("k"), None);
    }

    #[test]
    fn incr_creates_key() {
        let c = cache();
        assert_eq!(c.incr("counter"), Ok(1));
        assert_eq!(c.incr("counter"), Ok(2));
    }

    #[test]
    fn decr_key() {
        let c = cache();
        c.set("x", "10", None);
        assert_eq!(c.decr("x"), Ok(9));
        assert_eq!(c.decr("x"), Ok(8));
    }

    #[test]
    fn incrby_amount() {
        let c = cache();
        assert_eq!(c.incrby("n", 5), Ok(5));
        assert_eq!(c.incrby("n", 3), Ok(8));
        assert_eq!(c.incrby("n", -2), Ok(6));
    }

    #[test]
    fn incr_non_integer_errors() {
        let c = cache();
        c.set("s", "not_a_number", None);
        assert!(c.incr("s").is_err());
    }

    #[test]
    fn setnx_only_sets_if_missing() {
        let c = cache();
        assert!(c.setnx("k", "first", None));
        assert!(!c.setnx("k", "second", None));
        assert_eq!(c.get("k"), Some("first".to_string()));
    }

    #[test]
    fn getset_swaps_value() {
        let c = cache();
        c.set("k", "old", None);
        let old = c.getset("k", "new");
        assert_eq!(old, Some("old".to_string()));
        assert_eq!(c.get("k"), Some("new".to_string()));
    }

    #[test]
    fn getset_on_missing_key() {
        let c = cache();
        let old = c.getset("k", "val");
        assert_eq!(old, None);
        assert_eq!(c.get("k"), Some("val".to_string()));
    }

    #[test]
    fn mget_and_mset() {
        let c = cache();
        c.mset(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let vals = c.mget(&["a", "b", "missing", "c"]);
        assert_eq!(
            vals,
            vec![
                Some("1".to_string()),
                Some("2".to_string()),
                None,
                Some("3".to_string()),
            ]
        );
    }

    // -- TTL operations -----------------------------------------------------

    #[test]
    fn ttl_no_expiry() {
        let c = cache();
        c.set("k", "v", None);
        assert_eq!(c.ttl("k"), -1);
    }

    #[test]
    fn ttl_missing_key() {
        let c = cache();
        assert_eq!(c.ttl("nope"), -2);
    }

    #[test]
    fn expire_and_persist() {
        let c = cache();
        c.set("k", "v", None);
        assert!(c.expire("k", 100));
        assert!(c.ttl("k") > 0);
        assert!(c.persist("k"));
        assert_eq!(c.ttl("k"), -1);
    }

    #[test]
    fn expire_on_missing_key() {
        let c = cache();
        assert!(!c.expire("nope", 10));
    }

    // -- Del / Exists -------------------------------------------------------

    #[test]
    fn del_existing_key() {
        let c = cache();
        c.set("k", "v", None);
        assert!(c.del("k"));
        assert!(!c.exists("k"));
    }

    #[test]
    fn del_missing_key() {
        let c = cache();
        assert!(!c.del("nope"));
    }

    #[test]
    fn exists_key() {
        let c = cache();
        assert!(!c.exists("k"));
        c.set("k", "v", None);
        assert!(c.exists("k"));
    }

    // -- List operations ----------------------------------------------------

    #[test]
    fn lpush_and_rpush() {
        let c = cache();
        c.lpush("list", "b");
        c.lpush("list", "a");
        c.rpush("list", "c");
        assert_eq!(
            c.lrange("list", 0, -1),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn lpop_and_rpop() {
        let c = cache();
        c.rpush("list", "a");
        c.rpush("list", "b");
        c.rpush("list", "c");
        assert_eq!(c.lpop("list"), Some("a".to_string()));
        assert_eq!(c.rpop("list"), Some("c".to_string()));
        assert_eq!(c.llen("list"), 1);
    }

    #[test]
    fn lrange_with_negative_indices() {
        let c = cache();
        for v in &["a", "b", "c", "d", "e"] {
            c.rpush("list", v);
        }
        // Last two elements.
        assert_eq!(
            c.lrange("list", -2, -1),
            vec!["d".to_string(), "e".to_string()]
        );
    }

    #[test]
    fn llen_empty_and_missing() {
        let c = cache();
        assert_eq!(c.llen("nope"), 0);
        c.rpush("list", "x");
        assert_eq!(c.llen("list"), 1);
    }

    #[test]
    fn lpop_empty_list() {
        let c = cache();
        assert_eq!(c.lpop("nope"), None);
    }

    // -- Set operations -----------------------------------------------------

    #[test]
    fn sadd_and_sismember() {
        let c = cache();
        assert!(c.sadd("s", "a"));
        assert!(!c.sadd("s", "a")); // duplicate
        assert!(c.sismember("s", "a"));
        assert!(!c.sismember("s", "b"));
    }

    #[test]
    fn srem_member() {
        let c = cache();
        c.sadd("s", "a");
        c.sadd("s", "b");
        assert!(c.srem("s", "a"));
        assert!(!c.sismember("s", "a"));
        assert_eq!(c.scard("s"), 1);
    }

    #[test]
    fn smembers_returns_all() {
        let c = cache();
        c.sadd("s", "x");
        c.sadd("s", "y");
        let mut members = c.smembers("s");
        members.sort();
        assert_eq!(members, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn scard_and_empty() {
        let c = cache();
        assert_eq!(c.scard("nope"), 0);
        c.sadd("s", "a");
        assert_eq!(c.scard("s"), 1);
    }

    #[test]
    fn sinter_two_sets() {
        let c = cache();
        c.sadd("s1", "a");
        c.sadd("s1", "b");
        c.sadd("s1", "c");
        c.sadd("s2", "b");
        c.sadd("s2", "c");
        c.sadd("s2", "d");
        let mut inter = c.sinter("s1", "s2");
        inter.sort();
        assert_eq!(inter, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn sunion_two_sets() {
        let c = cache();
        c.sadd("s1", "a");
        c.sadd("s1", "b");
        c.sadd("s2", "b");
        c.sadd("s2", "c");
        let mut union = c.sunion("s1", "s2");
        union.sort();
        assert_eq!(
            union,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    // -- Hash operations ----------------------------------------------------

    #[test]
    fn hset_and_hget() {
        let c = cache();
        c.hset("h", "name", "alice");
        assert_eq!(c.hget("h", "name"), Some("alice".to_string()));
        assert_eq!(c.hget("h", "missing"), None);
    }

    #[test]
    fn hdel_field() {
        let c = cache();
        c.hset("h", "a", "1");
        c.hset("h", "b", "2");
        assert!(c.hdel("h", "a"));
        assert!(!c.hexists("h", "a"));
        assert_eq!(c.hlen("h"), 1);
    }

    #[test]
    fn hgetall_returns_map() {
        let c = cache();
        c.hset("h", "x", "1");
        c.hset("h", "y", "2");
        let all = c.hgetall("h");
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("x"), Some(&"1".to_string()));
        assert_eq!(all.get("y"), Some(&"2".to_string()));
    }

    #[test]
    fn hexists_and_hlen() {
        let c = cache();
        assert!(!c.hexists("h", "f"));
        assert_eq!(c.hlen("h"), 0);
        c.hset("h", "f", "v");
        assert!(c.hexists("h", "f"));
        assert_eq!(c.hlen("h"), 1);
    }

    #[test]
    fn hkeys_returns_field_names() {
        let c = cache();
        c.hset("h", "a", "1");
        c.hset("h", "b", "2");
        let mut keys = c.hkeys("h");
        keys.sort();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn hincrby_creates_and_increments() {
        let c = cache();
        assert_eq!(c.hincrby("h", "count", 5), Ok(5));
        assert_eq!(c.hincrby("h", "count", 3), Ok(8));
    }

    #[test]
    fn hincrby_non_integer_errors() {
        let c = cache();
        c.hset("h", "name", "alice");
        assert!(c.hincrby("h", "name", 1).is_err());
    }

    // -- Sorted set operations ----------------------------------------------

    #[test]
    fn zadd_and_zscore() {
        let c = cache();
        c.zadd("z", 1.5, "a");
        c.zadd("z", 2.5, "b");
        assert_eq!(c.zscore("z", "a"), Some(1.5));
        assert_eq!(c.zscore("z", "b"), Some(2.5));
        assert_eq!(c.zscore("z", "c"), None);
    }

    #[test]
    fn zrem_member() {
        let c = cache();
        c.zadd("z", 1.0, "a");
        c.zadd("z", 2.0, "b");
        assert!(c.zrem("z", "a"));
        assert!(!c.zrem("z", "a"));
        assert_eq!(c.zcard("z"), 1);
    }

    #[test]
    fn zrank_by_score() {
        let c = cache();
        c.zadd("z", 3.0, "c");
        c.zadd("z", 1.0, "a");
        c.zadd("z", 2.0, "b");
        assert_eq!(c.zrank("z", "a"), Some(0));
        assert_eq!(c.zrank("z", "b"), Some(1));
        assert_eq!(c.zrank("z", "c"), Some(2));
    }

    #[test]
    fn zrange_returns_ordered_slice() {
        let c = cache();
        c.zadd("z", 3.0, "c");
        c.zadd("z", 1.0, "a");
        c.zadd("z", 2.0, "b");
        let range = c.zrange("z", 0, 1);
        assert_eq!(
            range,
            vec![
                ("a".to_string(), 1.0),
                ("b".to_string(), 2.0),
            ]
        );
    }

    #[test]
    fn zcard_empty_and_filled() {
        let c = cache();
        assert_eq!(c.zcard("z"), 0);
        c.zadd("z", 1.0, "x");
        assert_eq!(c.zcard("z"), 1);
    }

    // -- Key pattern matching -----------------------------------------------

    #[test]
    fn keys_star_pattern() {
        let c = cache();
        c.set("user:1", "a", None);
        c.set("user:2", "b", None);
        c.set("post:1", "c", None);
        let mut matched = c.keys("user:*");
        matched.sort();
        assert_eq!(matched, vec!["user:1".to_string(), "user:2".to_string()]);
    }

    #[test]
    fn keys_question_mark_pattern() {
        let c = cache();
        c.set("a1", "v", None);
        c.set("a2", "v", None);
        c.set("ab", "v", None);
        let mut matched = c.keys("a?");
        matched.sort();
        assert_eq!(
            matched,
            vec!["a1".to_string(), "a2".to_string(), "ab".to_string()]
        );
    }

    #[test]
    fn keys_all_pattern() {
        let c = cache();
        c.set("x", "1", None);
        c.set("y", "2", None);
        assert_eq!(c.keys("*").len(), 2);
    }

    // -- Type detection -----------------------------------------------------

    #[test]
    fn key_type_detection() {
        let c = cache();
        c.set("s", "val", None);
        c.rpush("l", "item");
        c.sadd("set", "m");
        c.hset("h", "f", "v");
        c.zadd("z", 1.0, "m");

        assert_eq!(c.key_type("s"), Some("string"));
        assert_eq!(c.key_type("l"), Some("list"));
        assert_eq!(c.key_type("set"), Some("set"));
        assert_eq!(c.key_type("h"), Some("hash"));
        assert_eq!(c.key_type("z"), Some("zset"));
        assert_eq!(c.key_type("nope"), None);
    }

    // -- Eviction -----------------------------------------------------------

    #[test]
    fn lru_eviction_when_over_max_keys() {
        let c = CachePlugin::new(3);
        c.set("a", "1", None);
        c.set("b", "2", None);
        c.set("c", "3", None);

        // Access "a" so it becomes most-recently-used.
        c.get("a");

        // Adding a 4th key should evict the LRU key ("b" was set after "a"
        // but never accessed again, and "c" was set most recently).
        // Actually "b" has the oldest last_accessed because set() creates
        // entries with last_accessed = now, but "a" was accessed after "b"
        // and "c" was set after "b". So "b" should be evicted.
        c.set("d", "4", None);

        assert_eq!(c.dbsize(), 3);
        assert!(c.exists("a")); // was accessed, should survive
        assert!(!c.exists("b")); // LRU, should be evicted
        assert!(c.exists("c"));
        assert!(c.exists("d"));

        let stats = c.info();
        assert!(stats.evictions >= 1);
    }

    // -- Stats --------------------------------------------------------------

    #[test]
    fn stats_hit_miss_tracking() {
        let c = cache();
        c.set("k", "v", None);
        c.get("k"); // hit
        c.get("k"); // hit
        c.get("missing"); // miss

        let stats = c.info();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.sets, 1);
    }

    // -- Cleanup ------------------------------------------------------------

    #[test]
    fn cleanup_expired_keys() {
        let c = cache();
        c.set("keep", "yes", None);
        c.set("expire1", "no", Some(0));
        c.set("expire2", "no", Some(0));
        thread::sleep(Duration::from_millis(5));

        let removed = c.cleanup_expired();
        assert_eq!(removed, 2);
        assert!(c.exists("keep"));
        assert!(!c.exists("expire1"));
        assert!(!c.exists("expire2"));
    }

    // -- DBSIZE / FLUSHALL --------------------------------------------------

    #[test]
    fn dbsize_counts_non_expired() {
        let c = cache();
        c.set("a", "1", None);
        c.set("b", "2", None);
        c.set("c", "3", Some(0));
        thread::sleep(Duration::from_millis(5));
        // "c" is expired so dbsize should be 2.
        assert_eq!(c.dbsize(), 2);
    }

    #[test]
    fn flushall_clears_everything() {
        let c = cache();
        c.set("a", "1", None);
        c.set("b", "2", None);
        c.rpush("list", "x");
        c.flushall();
        assert_eq!(c.dbsize(), 0);
        let stats = c.info();
        assert_eq!(stats.sets, 0);
    }

    // -- Plugin trait -------------------------------------------------------

    #[test]
    fn plugin_name() {
        let c = cache();
        assert_eq!(Plugin::name(&c), "cache");
    }

    // -- Glob matching unit tests -------------------------------------------

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn glob_match_star() {
        assert!(glob_match("h*o", "hello"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("pre*", "prefix"));
        assert!(glob_match("*fix", "suffix"));
    }

    #[test]
    fn glob_match_question() {
        assert!(glob_match("h?llo", "hello"));
        assert!(!glob_match("h?llo", "hllo"));
    }
}
