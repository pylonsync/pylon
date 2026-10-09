//! Standalone benchmark, without building the runtime's other dependencies:
//! `rustc --edition=2021 -O crates/runtime/benches/rate_limit.rs -o /tmp/pylon-rate-limit-bench`
//! `/tmp/pylon-rate-limit-bench`
#[path = "../src/rate_limit.rs"]
mod rate_limit;

use std::hint::black_box;
use std::time::Instant;

fn main() {
    for cap in [100, 10_000] {
        let limiter = rate_limit::RateLimiter::new(cap, 3600);
        for _ in 0..cap {
            limiter.check("client").unwrap();
        }
        let mut times = Vec::new();
        for _ in 0..9 {
            let start = Instant::now();
            for _ in 0..1000 {
                assert!(black_box(limiter.check(black_box("client"))).is_err());
            }
            times.push(start.elapsed().as_secs_f64() * 1e6 / 1000.0);
        }
        times.sort_by(f64::total_cmp);
        assert_eq!(limiter.current_count("client"), cap);
        limiter.cleanup();
        println!(
            "{cap} active timestamps: {:.3} us/rejected request",
            times[4]
        );
    }
}
