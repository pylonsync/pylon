use std::time::Instant;

use pylon_plugin::builtin::cache::CachePlugin;

fn bench(name: &str, iterations: u32, mut f: impl FnMut()) {
    // Warmup.
    for _ in 0..100 {
        f();
    }

    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / iterations;
    let ops_sec = if per_op.as_nanos() > 0 {
        1_000_000_000 / per_op.as_nanos()
    } else {
        0
    };

    println!(
        "  {:<35} {:>8} ops  {:>10.2?} total  {:>8.2?}/op  {:>8} ops/sec",
        name, iterations, elapsed, per_op, ops_sec
    );
}

fn main() {
    println!("\npylon cache benchmarks\n");

    let cache = CachePlugin::new(1_000_000);

    // -----------------------------------------------------------------------
    // String operations
    // -----------------------------------------------------------------------

    println!("  --- Strings ---");

    bench("SET (no TTL)", 100_000, || {
        cache.set("bench_key", "hello world", None);
    });

    cache.set("bench_get", "hello world", None);
    bench("GET (hit)", 100_000, || {
        let _ = cache.get("bench_get");
    });

    bench("GET (miss)", 100_000, || {
        let _ = cache.get("nonexistent_key_xyz");
    });

    bench("SET + GET roundtrip", 100_000, || {
        cache.set("rt_key", "value", None);
        let _ = cache.get("rt_key");
    });

    bench("SETNX (exists)", 100_000, || {
        let _ = cache.setnx("bench_get", "new_value", None);
    });

    bench("INCR", 100_000, || {
        let _ = cache.incr("bench_counter");
    });

    bench("DECR", 100_000, || {
        let _ = cache.decr("bench_counter2");
    });

    // MSET + MGET
    let pairs: Vec<(&str, &str)> = vec![
        ("mk1", "v1"),
        ("mk2", "v2"),
        ("mk3", "v3"),
        ("mk4", "v4"),
        ("mk5", "v5"),
    ];
    bench("MSET (5 keys)", 50_000, || {
        cache.mset(&pairs);
    });

    let keys: Vec<&str> = vec!["mk1", "mk2", "mk3", "mk4", "mk5"];
    bench("MGET (5 keys)", 50_000, || {
        let _ = cache.mget(&keys);
    });

    bench("SET with TTL", 100_000, || {
        cache.set("ttl_key", "expires", Some(300));
    });

    bench("TTL check", 100_000, || {
        let _ = cache.ttl("ttl_key");
    });

    bench("EXISTS (hit)", 100_000, || {
        let _ = cache.exists("bench_get");
    });

    bench("DEL", 100_000, || {
        cache.set("del_key", "x", None);
        cache.del("del_key");
    });

    println!();

    // -----------------------------------------------------------------------
    // List operations
    // -----------------------------------------------------------------------

    println!("  --- Lists ---");

    bench("RPUSH", 100_000, || {
        let _ = cache.rpush("bench_list", "item");
    });

    // Reset list for fair benchmarks
    cache.del("bench_list2");
    for i in 0..1000 {
        cache.rpush("bench_list2", &format!("item_{i}"));
    }

    bench("LPUSH", 100_000, || {
        let _ = cache.lpush("bench_lpush", "item");
    });

    bench("RPOP", 50_000, || {
        cache.rpush("pop_list", "item");
        let _ = cache.rpop("pop_list");
    });

    bench("LPOP", 50_000, || {
        cache.rpush("lpop_list", "item");
        let _ = cache.lpop("lpop_list");
    });

    bench("LRANGE (0..99 of 1000)", 10_000, || {
        let _ = cache.lrange("bench_list2", 0, 99);
    });

    bench("LLEN", 100_000, || {
        let _ = cache.llen("bench_list2");
    });

    println!();

    // -----------------------------------------------------------------------
    // Set operations
    // -----------------------------------------------------------------------

    println!("  --- Sets ---");

    bench("SADD", 100_000, || {
        let _ = cache.sadd("bench_set", "member");
    });

    // Build a set with many members
    for i in 0..1000 {
        cache.sadd("big_set", &format!("member_{i}"));
    }

    bench("SISMEMBER (hit)", 100_000, || {
        let _ = cache.sismember("big_set", "member_500");
    });

    bench("SISMEMBER (miss)", 100_000, || {
        let _ = cache.sismember("big_set", "nonexistent");
    });

    bench("SCARD", 100_000, || {
        let _ = cache.scard("big_set");
    });

    // Build two sets for intersection
    for i in 0..500 {
        cache.sadd("set_a", &format!("m_{i}"));
        cache.sadd("set_b", &format!("m_{}", i + 250));
    }

    bench("SINTER (500 x 500)", 1_000, || {
        let _ = cache.sinter("set_a", "set_b");
    });

    bench("SUNION (500 x 500)", 1_000, || {
        let _ = cache.sunion("set_a", "set_b");
    });

    bench("SMEMBERS (1000)", 1_000, || {
        let _ = cache.smembers("big_set");
    });

    println!();

    // -----------------------------------------------------------------------
    // Hash operations
    // -----------------------------------------------------------------------

    println!("  --- Hashes ---");

    bench("HSET", 100_000, || {
        cache.hset("bench_hash", "field1", "value1");
    });

    // Build a hash with many fields
    for i in 0..100 {
        cache.hset("big_hash", &format!("field_{i}"), &format!("value_{i}"));
    }

    bench("HGET (hit)", 100_000, || {
        let _ = cache.hget("big_hash", "field_50");
    });

    bench("HGET (miss)", 100_000, || {
        let _ = cache.hget("big_hash", "nonexistent_field");
    });

    bench("HEXISTS", 100_000, || {
        let _ = cache.hexists("big_hash", "field_50");
    });

    bench("HLEN", 100_000, || {
        let _ = cache.hlen("big_hash");
    });

    bench("HGETALL (100 fields)", 10_000, || {
        let _ = cache.hgetall("big_hash");
    });

    bench("HKEYS (100 fields)", 10_000, || {
        let _ = cache.hkeys("big_hash");
    });

    bench("HINCRBY", 100_000, || {
        let _ = cache.hincrby("bench_hash", "counter", 1);
    });

    println!();

    // -----------------------------------------------------------------------
    // Sorted set operations
    // -----------------------------------------------------------------------

    println!("  --- Sorted Sets ---");

    bench("ZADD", 100_000, || {
        cache.zadd("bench_zset", 1.0, "member");
    });

    // Build a sorted set
    for i in 0..1000 {
        cache.zadd("big_zset", i as f64, &format!("player_{i}"));
    }

    bench("ZSCORE (hit)", 100_000, || {
        let _ = cache.zscore("big_zset", "player_500");
    });

    bench("ZRANK", 10_000, || {
        let _ = cache.zrank("big_zset", "player_500");
    });

    bench("ZRANGE (top 10 of 1000)", 10_000, || {
        let _ = cache.zrange("big_zset", 0, 9);
    });

    bench("ZCARD", 100_000, || {
        let _ = cache.zcard("big_zset");
    });

    println!();

    // -----------------------------------------------------------------------
    // Key operations
    // -----------------------------------------------------------------------

    println!("  --- Keys / Utility ---");

    bench("KEYS (pattern: bench_*)", 1_000, || {
        let _ = cache.keys("bench_*");
    });

    bench("DBSIZE", 100_000, || {
        let _ = cache.dbsize();
    });

    bench("TYPE", 100_000, || {
        let _ = cache.key_type("bench_get");
    });

    bench("INFO (stats)", 100_000, || {
        let _ = cache.info();
    });

    println!();

    // -----------------------------------------------------------------------
    // Eviction benchmark
    // -----------------------------------------------------------------------

    println!("  --- Eviction ---");

    let small_cache = CachePlugin::new(1_000);
    // Fill to capacity
    for i in 0..1000 {
        small_cache.set(&format!("ek_{i}"), "value", None);
    }

    bench("SET (triggers LRU eviction)", 10_000, || {
        small_cache.set("overflow_key", "value", None);
    });

    println!();

    // -----------------------------------------------------------------------
    // Mixed workload
    // -----------------------------------------------------------------------

    println!("  --- Mixed Workload ---");

    let mixed = CachePlugin::new(100_000);
    let mut counter = 0u64;

    bench("80% read / 20% write (string)", 100_000, || {
        counter += 1;
        if counter % 5 == 0 {
            mixed.set(&format!("k_{}", counter % 1000), "value", None);
        } else {
            let _ = mixed.get(&format!("k_{}", counter % 1000));
        }
    });

    bench("Session store pattern (set+get+ttl)", 50_000, || {
        counter += 1;
        let key = format!("session_{}", counter % 5000);
        mixed.set(&key, "user_data", Some(3600));
        let _ = mixed.get(&key);
        let _ = mixed.ttl(&key);
    });

    bench("Counter pattern (incr+get)", 100_000, || {
        counter += 1;
        let key = format!("counter_{}", counter % 100);
        let _ = mixed.incr(&key);
    });

    bench("Leaderboard pattern (zadd+zrange)", 10_000, || {
        counter += 1;
        mixed.zadd(
            "leaderboard",
            counter as f64,
            &format!("player_{}", counter % 100),
        );
        let _ = mixed.zrange("leaderboard", 0, 9);
    });

    println!();

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    let stats = cache.info();
    println!("  Cache stats after benchmark:");
    println!("    Keys:      {}", cache.dbsize());
    println!("    Hits:      {}", stats.hits);
    println!("    Misses:    {}", stats.misses);
    println!("    Sets:      {}", stats.sets);
    println!("    Evictions: {}", stats.evictions);
    println!();
}
