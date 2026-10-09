//! Run with `cargo bench -p pylon-plugin --bench cache_reads`.
use std::hint::black_box;
use std::time::Instant;

use pylon_plugin::builtin::cache::CachePlugin;

fn measure(name: &str, mut op: impl FnMut()) {
    for _ in 0..100 {
        op();
    }
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..1000 {
            op();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!("{name}: {:.2} us/op", samples[4]);
}

fn main() {
    for size in [100, 100_000] {
        let cache = CachePlugin::new(size);
        for id in 0..size {
            cache.set(&format!("key-{id}"), "value", None);
        }
        measure(&format!("GET, {size} keys"), || {
            black_box(cache.get(black_box("key-0")));
        });
        measure(&format!("SET existing, {size} keys"), || {
            cache.set(black_box("key-0"), "value", None);
        });
    }
    let cache = CachePlugin::new(100);
    for id in 0..10_000 {
        let member = format!("member-{id:05}");
        cache.zadd("scores", (id % 100) as f64, &member);
        cache.sadd("all", &member);
        if id < 10 {
            cache.sadd("small", &member);
        }
    }
    measure("ZRANGE first 10 of 10,000", || {
        black_box(cache.zrange("scores", 0, 9));
    });
    measure("SINTER 10,000 and 10", || {
        black_box(cache.sinter("all", "small"));
    });
}
