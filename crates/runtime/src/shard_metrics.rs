//! Shard numbers on `/metrics`: Prometheus text and the JSON Studio reads.
//!
//! Every metric carries `shard` and `kind` labels. Percentiles cover the
//! shard's last minute of ticks (see `pylon_realtime::stats`); counters run
//! from the shard's start.
//!
//! An app that makes a shard per match makes new series for every match.
//! `PYLON_SHARD_METRICS_LABELS=kind` reports one series per kind instead:
//! counters and gauges are summed, and each percentile is the highest any
//! shard of the kind has (the worst shard). The JSON stays per shard.

use std::fmt::Write as _;
use std::sync::Arc;

use pylon_realtime::{DynShardRegistry, ShardStats};

/// Per-kind rows instead of per-shard ones: counters and gauges summed,
/// percentiles the highest of the kind's shards.
fn by_kind(rows: Vec<(String, String, ShardStats)>) -> Vec<(String, ShardStats)> {
    let mut kinds: std::collections::BTreeMap<String, ShardStats> = Default::default();
    for (_, kind, s) in rows {
        let t = kinds.entry(kind).or_default();
        t.ticks += s.ticks;
        t.overruns += s.overruns;
        t.subscribers += s.subscribers;
        t.input_queue += s.input_queue;
        t.bytes_total += s.bytes_total;
        t.dropped_frames_total += s.dropped_frames_total;
        t.dropped_inputs_total.queue_full += s.dropped_inputs_total.queue_full;
        t.dropped_inputs_total.rate_limited += s.dropped_inputs_total.rate_limited;
        let (a, b) = (&mut t.phase_seconds_total, s.phase_seconds_total);
        a.whole += b.whole;
        a.inputs += b.inputs;
        a.tick += b.tick;
        a.interest += b.interest;
        a.encode += b.encode;
        for (mine, theirs) in [
            (&mut t.tick_ms, s.tick_ms),
            (&mut t.inputs_ms, s.inputs_ms),
            (&mut t.sim_ms, s.sim_ms),
            (&mut t.interest_ms, s.interest_ms),
            (&mut t.encode_ms, s.encode_ms),
            (&mut t.bytes_per_tick, s.bytes_per_tick),
            (&mut t.bytes_per_subscriber, s.bytes_per_subscriber),
        ] {
            mine.p50 = mine.p50.max(theirs.p50);
            mine.p99 = mine.p99.max(theirs.p99);
            mine.max = mine.max.max(theirs.max);
        }
    }
    kinds.into_iter().collect()
}

/// True when `PYLON_SHARD_METRICS_LABELS=kind`.
fn per_kind() -> bool {
    std::env::var("PYLON_SHARD_METRICS_LABELS").is_ok_and(|v| v.trim() == "kind")
}

/// Each shard with its kind and numbers, sorted by id.
fn collect(registry: &dyn DynShardRegistry) -> Vec<(String, String, ShardStats)> {
    let mut ids = registry.ids();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            let shard = registry.get(&id)?;
            let kind = registry.kind(&id).unwrap_or_default();
            Some((id, kind, shard.stats()))
        })
        .collect()
}

/// A Prometheus label value: backslash, quote, and newline escaped.
fn label(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

struct Family<'a> {
    out: &'a mut String,
    name: &'static str,
}

impl<'a> Family<'a> {
    fn new(out: &'a mut String, name: &'static str, kind: &str, help: &str) -> Self {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} {kind}");
        Self { out, name }
    }

    fn sample(&mut self, suffix: &str, labels: &str, extra: &str, value: f64) {
        let sep = if extra.is_empty() { "" } else { "," };
        let _ = writeln!(
            self.out,
            "{}{suffix}{{{labels}{sep}{extra}}} {value}",
            self.name
        );
    }
}

/// The Prometheus exposition of every shard's numbers.
pub fn prometheus(registry: &dyn DynShardRegistry) -> String {
    render(collect(registry), per_kind())
}

fn render(shards: Vec<(String, String, ShardStats)>, per_kind: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# HELP pylon_shards Shards in the registry.");
    let _ = writeln!(out, "# TYPE pylon_shards gauge");
    let _ = writeln!(out, "pylon_shards {}", shards.len());
    if shards.is_empty() {
        return out;
    }
    let series: Vec<(String, ShardStats)> = if per_kind {
        by_kind(shards)
            .into_iter()
            .map(|(kind, s)| (format!("kind=\"{}\"", label(&kind)), s))
            .collect()
    } else {
        shards
            .into_iter()
            .map(|(id, kind, s)| {
                (
                    format!("shard=\"{}\",kind=\"{}\"", label(&id), label(&kind)),
                    s,
                )
            })
            .collect()
    };
    let rows = || series.iter().map(|(l, s)| (l, s));

    let mut f = Family::new(
        &mut out,
        "pylon_shard_subscribers",
        "gauge",
        "Live subscriber connections.",
    );
    for (l, s) in rows() {
        f.sample("", l, "", s.subscribers as f64);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_input_queue",
        "gauge",
        "Inputs waiting for the next tick.",
    );
    for (l, s) in rows() {
        f.sample("", l, "", s.input_queue as f64);
    }
    let mut f = Family::new(&mut out, "pylon_shard_ticks_total", "counter", "Ticks run.");
    for (l, s) in rows() {
        f.sample("", l, "", s.ticks as f64);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_tick_overruns_total",
        "counter",
        "Ticks skipped because the loop fell behind.",
    );
    for (l, s) in rows() {
        f.sample("", l, "", s.overruns as f64);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_tick_duration_seconds",
        "summary",
        "Tick duration over the last minute of ticks.",
    );
    for (l, s) in rows() {
        f.sample("", l, "quantile=\"0.5\"", s.tick_ms.p50 / 1000.0);
        f.sample("", l, "quantile=\"0.99\"", s.tick_ms.p99 / 1000.0);
        f.sample("_sum", l, "", s.phase_seconds_total.whole);
        f.sample("_count", l, "", s.ticks as f64);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_phase_seconds_total",
        "counter",
        "Time spent per tick phase: inputs (apply_input), tick, interest (interest management and snapshot building), encode (encoding and queueing frames).",
    );
    for (l, s) in rows() {
        let p = s.phase_seconds_total;
        f.sample("", l, "phase=\"inputs\"", p.inputs);
        f.sample("", l, "phase=\"tick\"", p.tick);
        f.sample("", l, "phase=\"interest\"", p.interest);
        f.sample("", l, "phase=\"encode\"", p.encode);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_phase_duration_seconds",
        "gauge",
        "Tick phase duration percentiles over the last minute of ticks.",
    );
    for (l, s) in rows() {
        for (phase, q) in [
            ("inputs", s.inputs_ms),
            ("tick", s.sim_ms),
            ("interest", s.interest_ms),
            ("encode", s.encode_ms),
        ] {
            f.sample(
                "",
                l,
                &format!("phase=\"{phase}\",quantile=\"0.5\""),
                q.p50 / 1000.0,
            );
            f.sample(
                "",
                l,
                &format!("phase=\"{phase}\",quantile=\"0.99\""),
                q.p99 / 1000.0,
            );
        }
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_frame_bytes_total",
        "counter",
        "Bytes of frames queued for subscribers.",
    );
    for (l, s) in rows() {
        f.sample("", l, "", s.bytes_total as f64);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_tick_bytes",
        "gauge",
        "Frame bytes queued per tick for all subscribers, percentiles over the last minute of ticks.",
    );
    for (l, s) in rows() {
        f.sample("", l, "quantile=\"0.5\"", s.bytes_per_tick.p50);
        f.sample("", l, "quantile=\"0.99\"", s.bytes_per_tick.p99);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_subscriber_tick_bytes",
        "gauge",
        "Frame bytes queued per subscriber per tick, percentiles over recent ticks.",
    );
    for (l, s) in rows() {
        f.sample("", l, "quantile=\"0.5\"", s.bytes_per_subscriber.p50);
        f.sample("", l, "quantile=\"0.99\"", s.bytes_per_subscriber.p99);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_dropped_frames_total",
        "counter",
        "Frames dropped from full subscriber queues.",
    );
    for (l, s) in rows() {
        f.sample("", l, "", s.dropped_frames_total as f64);
    }
    let mut f = Family::new(
        &mut out,
        "pylon_shard_dropped_inputs_total",
        "counter",
        "Inputs refused before queueing: the shard's queue was full, or the subscriber was over its rate limit.",
    );
    for (l, s) in rows() {
        let d = s.dropped_inputs_total;
        f.sample("", l, "reason=\"queue_full\"", d.queue_full as f64);
        f.sample("", l, "reason=\"rate_limited\"", d.rate_limited as f64);
    }
    out
}

/// The shards section of the `/metrics` JSON.
pub fn json(registry: &dyn DynShardRegistry) -> serde_json::Value {
    collect(registry)
        .into_iter()
        .map(|(id, kind, stats)| serde_json::json!({ "id": id, "kind": kind, "stats": stats }))
        .collect()
}

/// `prometheus` for an optional registry.
pub fn prometheus_opt(registry: &Option<Arc<dyn DynShardRegistry>>) -> String {
    registry.as_deref().map(prometheus).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_realtime::{Shard, ShardConfig, ShardRegistry, SimState};

    #[derive(Default)]
    struct Counter(u64);

    impl SimState for Counter {
        type Input = u64;
        type Snapshot = u64;
        type Error = String;
        fn apply_input(
            &mut self,
            _: &pylon_realtime::SubscriberId,
            input: u64,
            _: std::time::Instant,
        ) -> Result<(), String> {
            self.0 += input;
            Ok(())
        }
        fn tick(&mut self, _: std::time::Duration) {}
        fn snapshot(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn every_family_has_a_sample_for_a_running_shard() {
        let registry = ShardRegistry::<Counter>::new();
        let shard = Shard::new("zone \"1\"", Counter::default(), ShardConfig::default());
        shard.run_tick();
        shard.run_tick();
        registry.insert(shard);
        let text = prometheus(&registry);
        assert!(text.contains("pylon_shards 1"));
        for family in [
            "pylon_shard_subscribers",
            "pylon_shard_input_queue",
            "pylon_shard_ticks_total",
            "pylon_shard_tick_overruns_total",
            "pylon_shard_tick_duration_seconds",
            "pylon_shard_phase_seconds_total",
            "pylon_shard_phase_duration_seconds",
            "pylon_shard_frame_bytes_total",
            "pylon_shard_tick_bytes",
            "pylon_shard_subscriber_tick_bytes",
            "pylon_shard_dropped_frames_total",
            "pylon_shard_dropped_inputs_total",
        ] {
            assert!(text.contains(&format!("# TYPE {family} ")), "{family}");
            assert!(
                text.contains(&format!("{family}{{shard=\"zone \\\"1\\\"\",kind=\"\"")),
                "{family} sample in\n{text}"
            );
        }
        // The registry runs the shard's tick loop too: at least the two.
        let ticks: f64 = text
            .lines()
            .find(|l| l.starts_with("pylon_shard_ticks_total{"))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse().ok())
            .unwrap();
        assert!(ticks >= 2.0, "{ticks}");
        let json = json(&registry);
        assert!(json[0]["stats"]["ticks"].as_u64().unwrap() >= 2);

        // Per kind: one series, the counters summed across shards.
        let mut a = ShardStats {
            ticks: 3,
            ..Default::default()
        };
        a.tick_ms.p99 = 2.0;
        let mut b = ShardStats {
            ticks: 4,
            ..a.clone()
        };
        b.tick_ms.p99 = 5.0;
        let text = render(
            vec![
                ("m1".into(), "arena".into(), a),
                ("m2".into(), "arena".into(), b),
            ],
            true,
        );
        assert!(
            text.contains("pylon_shard_ticks_total{kind=\"arena\"} 7"),
            "{text}"
        );
        assert!(text
            .contains("pylon_shard_tick_duration_seconds{kind=\"arena\",quantile=\"0.99\"} 0.005"));
        assert!(!text.contains("shard=\"m1\""));
        assert_eq!(json[0]["id"], "zone \"1\"");
    }
}
