use std::collections::VecDeque;

use pylon_plugin::builtin::cache::CachePlugin;

#[test]
fn cache_eviction_matches_a_reference_access_order() {
    for capacity in [1, 3, 32] {
        let cache = CachePlugin::new(capacity);
        let mut order: VecDeque<String> = VecDeque::new();
        let mut rng = 12345u64;
        for _ in 0..2000 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = format!("key-{}", (rng >> 32) % 64);
            let index = order.iter().position(|k| k == &key);
            match rng % 4 {
                0 | 1 => {
                    cache.set(&key, "value", None);
                    if let Some(i) = index {
                        order.remove(i);
                    } else if order.len() == capacity {
                        order.pop_front();
                    }
                    order.push_back(key);
                }
                2 => {
                    assert_eq!(cache.get(&key), index.map(|_| "value".to_string()));
                    if let Some(i) = index {
                        order.remove(i);
                        order.push_back(key);
                    }
                }
                _ => {
                    assert_eq!(cache.del(&key), index.is_some());
                    if let Some(i) = index {
                        order.remove(i);
                    }
                }
            }
            assert_eq!(cache.dbsize(), order.len());
            for id in 0..64 {
                let key = format!("key-{id}");
                assert_eq!(cache.exists(&key), order.contains(&key));
            }
        }
    }
}

#[test]
fn expiry_replacement_batches_and_flush_preserve_eviction_order() {
    let cache = CachePlugin::new(3);
    cache.set("expired", "value", Some(0));
    cache.set("b", "value", None);
    cache.set("c", "value", None);
    assert_eq!(cache.get("expired"), None);
    cache.set("d", "value", None);
    cache.mset(&[("b", "one"), ("b", "two"), ("c", "three")]);
    cache.set("e", "value", None);
    assert!(!cache.exists("d"));
    assert_eq!(cache.get("b").as_deref(), Some("two"));
    assert_eq!(cache.get("c").as_deref(), Some("three"));
    assert_eq!(cache.dbsize(), 3);
    cache.flushall();
    assert_eq!(cache.dbsize(), 0);
    cache.mset(&[("x", "1"), ("y", "2"), ("z", "3"), ("w", "4")]);
    assert!(!cache.exists("x"));
    assert_eq!(cache.dbsize(), 3);
}

#[test]
fn set_operations_handle_aliases_missing_keys_and_other_types() {
    let cache = CachePlugin::new(10);
    cache.sadd("a", "one");
    cache.sadd("a", "two");
    cache.sadd("b", "two");
    cache.sadd("b", "three");
    cache.set("wrong-type", "value", None);
    cache.sadd("expired", "gone");
    cache.expire("expired", 0);
    for absent in ["missing", "wrong-type", "expired"] {
        assert!(cache.sinter("a", absent).is_empty());
        assert!(cache.sinter(absent, "a").is_empty());
        for pair in [("a", absent), (absent, "a")] {
            let mut union = cache.sunion(pair.0, pair.1);
            union.sort();
            assert_eq!(union, ["one", "two"]);
        }
    }
    assert_eq!(cache.sinter("a", "b"), ["two"]);
    let mut same = cache.sinter("a", "a");
    same.sort();
    assert_eq!(same, ["one", "two"]);
    let mut union = cache.sunion("a", "a");
    union.sort();
    assert_eq!(union, same);
}

#[test]
fn sorted_ranges_preserve_ties_and_handle_empty_or_out_of_bounds_pages() {
    let cache = CachePlugin::new(10);
    cache.zadd("empty", 1.0, "only");
    cache.zrem("empty", "only");
    assert!(cache.zrange("empty", 0, usize::MAX).is_empty());
    let mut expected = Vec::new();
    for id in 0..256 {
        let member = format!("member-{id:03}");
        let score = ((id * 37) % 17) as f64 - 8.0;
        cache.zadd("scores", score, &member);
        expected.push((member, score));
    }
    expected.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then_with(|| a.0.cmp(&b.0)));
    for start in (0..300).step_by(13) {
        let wanted: Vec<_> = expected.iter().skip(start).take(10).cloned().collect();
        assert_eq!(cache.zrange("scores", start, start + 9), wanted);
    }
    assert!(cache.zrange("scores", 20, 19).is_empty());
    assert_eq!(cache.zrange("scores", 0, usize::MAX), expected);
}
