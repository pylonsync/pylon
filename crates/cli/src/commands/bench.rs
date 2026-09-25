//! `pylon bench shard` — put N headless clients into a realtime shard and
//! report what they saw.
//!
//! Each bot joins the way a player does, then sends inputs at a set rate
//! and decodes every frame with the shard wire protocol (version 2):
//!
//! - `--join <function>`: a guest session (`POST /api/auth/guest`), then the
//!   app's join function, which returns `{ shardId, subscriberId, ticket }`.
//! - `--shard <id>`: connect to a running shard with tickets minted here.
//!   The key is `PYLON_SHARD_TICKET_SECRET`, or the one `pylon dev` derives
//!   in this directory, so run it from the app directory for a local shard.
//!
//! The report gives the tick rate each bot saw, the interval between
//! snapshots (and its jitter), input-to-ack latency, bytes received per
//! bot, and input rejections. `--report <file>` writes a JSON report that
//! `pylon bench merge` combines, so bots can run from several machines.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use pylon_kernel::ExitCode;
use pylon_realtime::wire::{self, InputRejection};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::output;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .skip_while(|a| *a != "bench")
        .skip(1)
        .collect();
    match rest.first().copied() {
        Some("shard") => match BenchConfig::parse(&rest[1..]) {
            Ok(config) => run_shard(config, json_mode),
            Err(e) => {
                output::print_error(&e);
                eprintln!("{USAGE}");
                ExitCode::Usage
            }
        },
        Some("merge") => run_merge(&rest[1..], json_mode),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::Usage
        }
    }
}

pub const USAGE: &str = "\
Usage:
  pylon bench shard (--join <function> | --shard <id>) [options]
  pylon bench merge <report.json>...

Options:
  --url <origin>       App origin (default http://localhost:4321)
  --bots <n>           Bots to run (default 100)
  --duration <secs>    How long each bot stays connected (default 30)
  --ramp <secs>        Spread the bots' joins over this long (default 5)
  --rate <n>           Inputs per bot per second (default 10; 0 = none)
  --input <json>       Input template, repeatable (cycled). String values
                       \"$rand:A:B\" (number), \"$int:A:B\", \"$bot\", \"$sid\"
                       are replaced per input.
  --join <function>    Join like a player: guest session, then this function,
                       which returns { shardId, subscriberId, ticket }
  --shard <id>         Join this running shard with locally minted tickets
  --sid-prefix <p>     Subscriber ids for --shard (default \"bot-\")
  --bot-offset <n>     First bot number, for runs on several machines
  --report <file>      Write a JSON report for `pylon bench merge`
  --json               Print the report as JSON";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum JoinMode {
    Function(String),
    Shard(String),
}

#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub url: String,
    pub bots: usize,
    pub duration: Duration,
    pub ramp: Duration,
    pub rate: f64,
    pub inputs: Vec<serde_json::Value>,
    pub join: JoinMode,
    pub sid_prefix: String,
    pub bot_offset: usize,
    pub report: Option<String>,
}

impl BenchConfig {
    pub fn parse(args: &[&str]) -> Result<Self, String> {
        let mut url = "http://localhost:4321".to_string();
        let mut bots = 100usize;
        let mut duration = 30.0f64;
        let mut ramp = 5.0f64;
        let mut rate = 10.0f64;
        let mut inputs = Vec::new();
        let mut join = None;
        let mut sid_prefix = "bot-".to_string();
        let mut bot_offset = 0usize;
        let mut report = None;
        let mut i = 0;
        let value = |i: usize, flag: &str| -> Result<String, String> {
            args.get(i + 1)
                .map(|v| v.to_string())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        let number = |v: String, flag: &str| -> Result<f64, String> {
            v.parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| format!("{flag} must be a non-negative number, got {v}"))
        };
        while i < args.len() {
            let flag = args[i];
            match flag {
                "--url" => url = value(i, flag)?.trim_end_matches('/').to_string(),
                "--bots" => bots = number(value(i, flag)?, flag)? as usize,
                "--duration" => duration = number(value(i, flag)?, flag)?,
                "--ramp" => ramp = number(value(i, flag)?, flag)?,
                "--rate" => rate = number(value(i, flag)?, flag)?,
                "--input" => {
                    let raw = value(i, flag)?;
                    inputs.push(
                        serde_json::from_str(&raw)
                            .map_err(|e| format!("--input is not JSON ({e}): {raw}"))?,
                    );
                }
                "--join" => join = Some(JoinMode::Function(value(i, flag)?)),
                "--shard" => join = Some(JoinMode::Shard(value(i, flag)?)),
                "--sid-prefix" => sid_prefix = value(i, flag)?,
                "--bot-offset" => bot_offset = number(value(i, flag)?, flag)? as usize,
                "--report" => report = Some(value(i, flag)?),
                "--json" => {
                    i += 1;
                    continue;
                }
                other => return Err(format!("unknown option {other}")),
            }
            i += 2;
        }
        let join = join.ok_or("pass --join <function> or --shard <id>")?;
        if bots == 0 || bots > 100_000 {
            return Err("--bots must be from 1 to 100000".into());
        }
        // A week is far past any load test and far inside Duration's range.
        const MAX_SECS: f64 = 7.0 * 24.0 * 3600.0;
        if duration > MAX_SECS || ramp > MAX_SECS {
            return Err("--duration and --ramp must be at most a week".into());
        }
        if rate > 1000.0 {
            return Err("--rate must be at most 1000 inputs per second".into());
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!(
                "--url must start with http:// or https://, got {url}"
            ));
        }
        if rate > 0.0 && inputs.is_empty() {
            return Err("--rate needs at least one --input template (or pass --rate 0)".into());
        }
        Ok(Self {
            url,
            bots,
            duration: Duration::from_secs_f64(duration),
            ramp: Duration::from_secs_f64(ramp),
            rate,
            inputs,
            join,
            sid_prefix,
            bot_offset,
            report,
        })
    }

    fn ws_base(&self) -> String {
        if let Some(rest) = self.url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else {
            format!("ws://{}", self.url.trim_start_matches("http://"))
        }
    }
}

/// Fill an input template for one input. See `--input`.
pub fn expand_template(
    template: &serde_json::Value,
    bot: usize,
    sid: &str,
    rng: &mut impl rand::Rng,
) -> serde_json::Value {
    use serde_json::Value;
    match template {
        Value::String(s) => {
            if s == "$bot" {
                return Value::from(bot as u64);
            }
            if s == "$sid" {
                return Value::from(sid);
            }
            let range = |rest: &str| -> Option<(f64, f64)> {
                let (a, b) = rest.split_once(':')?;
                let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
                (a <= b).then_some((a, b))
            };
            if let Some((a, b)) = s.strip_prefix("$rand:").and_then(range) {
                return Value::from(rng.gen_range(a..=b));
            }
            if let Some((a, b)) = s.strip_prefix("$int:").and_then(range) {
                return Value::from(rng.gen_range(a as i64..=b as i64));
            }
            template.clone()
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| expand_template(v, bot, sid, rng))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), expand_template(v, bot, sid, rng)))
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Histogram: log-linear buckets, mergeable across machines
// ---------------------------------------------------------------------------

/// Sub-buckets per power of two: about 6% precision.
const SUB_BUCKETS: u64 = 16;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Histogram {
    /// Bucket index to count.
    pub buckets: BTreeMap<u32, u64>,
    pub count: u64,
    pub max: u64,
    /// The exact smallest value recorded (the buckets only know ranges).
    #[serde(default)]
    pub min: Option<u64>,
}

impl Histogram {
    fn bucket_of(v: u64) -> u32 {
        if v < SUB_BUCKETS {
            return v as u32;
        }
        let exp = 63 - v.leading_zeros() as u64; // floor(log2 v), >= 4
        let shift = exp - 4; // SUB_BUCKETS = 2^4
        let sub = (v >> shift) - SUB_BUCKETS; // 0..16
        ((shift + 1) * SUB_BUCKETS + sub) as u32
    }

    /// The highest bucket index a u64 falls in.
    fn max_bucket() -> u32 {
        Self::bucket_of(u64::MAX)
    }

    /// The midpoint of a bucket.
    fn mid_of(bucket: u32) -> u64 {
        let low = match bucket {
            0 => 0,
            b => Self::value_of(b - 1) as u128 + 1,
        };
        ((low + Self::value_of(bucket) as u128) / 2) as u64
    }

    /// The upper edge of a bucket. Indexes past the last u64 bucket clamp.
    fn value_of(bucket: u32) -> u64 {
        let b = bucket.min(Self::max_bucket()) as u64;
        if b < SUB_BUCKETS {
            return b;
        }
        let shift = b / SUB_BUCKETS - 1;
        let sub = b % SUB_BUCKETS;
        let edge = ((SUB_BUCKETS + sub + 1) as u128) << shift;
        (edge - 1).min(u64::MAX as u128) as u64
    }

    /// Reject a histogram read from a file that could not come from
    /// `record`: bucket indexes out of range, or counts that do not add up.
    pub fn validate(&self) -> Result<(), String> {
        let mut total: u64 = 0;
        for (b, c) in &self.buckets {
            if *b > Self::max_bucket() {
                return Err(format!("bucket {b} is out of range"));
            }
            total = total.checked_add(*c).ok_or("bucket counts overflow")?;
        }
        if total != self.count {
            return Err(format!(
                "buckets hold {total} values, count says {}",
                self.count
            ));
        }
        if let Some(min) = self.min {
            if self.count == 0 || min > self.max {
                return Err("min does not match the recorded values".into());
            }
        }
        Ok(())
    }

    pub fn record(&mut self, v: u64) {
        *self.buckets.entry(Self::bucket_of(v)).or_insert(0) += 1;
        self.count += 1;
        self.max = self.max.max(v);
        self.min = Some(self.min.map_or(v, |m| m.min(v)));
    }

    /// Add `other`'s values. Fails, leaving `self` unchanged, when the
    /// total count would overflow.
    pub fn merge(&mut self, other: &Histogram) -> Result<(), String> {
        let count = self
            .count
            .checked_add(other.count)
            .ok_or("merged histogram holds more than 2^64 values")?;
        for (b, c) in &other.buckets {
            let slot = self.buckets.entry(*b).or_insert(0);
            // Each bucket is at most the total, which did not overflow.
            *slot += c;
        }
        // An empty side is neutral. A non-empty side without a minimum (a
        // report from before minimums were stored) makes it unknown.
        self.min = match (self.count, self.min, other.count, other.min) {
            (0, _, _, m) => m,
            (_, m, 0, _) => m,
            (_, Some(a), _, Some(b)) => Some(a.min(b)),
            _ => None,
        };
        self.count = count;
        self.max = self.max.max(other.max);
        Ok(())
    }

    /// The value at quantile `q` (0..=1), within the bucket precision.
    pub fn quantile(&self, q: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        if q >= 1.0 {
            return Some(self.max);
        }
        if q <= 0.0 {
            if let Some(min) = self.min {
                return Some(min);
            }
        }
        let rank = ((q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64).max(1);
        let mut seen = 0;
        for (b, c) in &self.buckets {
            seen += c;
            if seen >= rank {
                let floor = self.min.unwrap_or(0).min(self.max);
                return Some(Self::mid_of(*b).clamp(floor, self.max));
            }
        }
        Some(self.max)
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub runs: Vec<RunInfo>,
    pub bots: u64,
    pub connected: u64,
    pub failed: u64,
    /// Bots whose connection closed before the run ended.
    pub dropped: u64,
    /// Why joins or connections failed, with counts.
    pub errors: BTreeMap<String, u64>,
    pub frames: u64,
    pub inputs_sent: u64,
    pub inputs_acked: u64,
    /// Bots that got at least one of their inputs acked.
    #[serde(default)]
    pub bots_acked: u64,
    /// Inputs dropped from the ack wait list because it hit its cap (the
    /// server read them but never acked them).
    #[serde(default)]
    pub inputs_unacked_dropped: u64,
    pub rejections: BTreeMap<String, u64>,
    pub decode_errors: u64,
    /// Snapshot interval seen by a bot, microseconds.
    pub snapshot_interval_us: Histogram,
    /// Input send to the first frame that acks it, microseconds.
    pub ack_latency_us: Histogram,
    /// Per bot: distinct ticks per second, in milli-hertz.
    pub tick_rate_mhz: Histogram,
    /// Per bot: bytes received per second.
    pub bytes_per_sec: Histogram,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunInfo {
    pub url: String,
    pub bots: u64,
    pub bot_offset: u64,
    pub duration_secs: f64,
    pub rate: f64,
}

impl Report {
    pub fn merge(&mut self, other: &Report) -> Result<(), String> {
        self.runs.extend(other.runs.iter().cloned());
        self.bots += other.bots;
        self.connected += other.connected;
        self.failed += other.failed;
        self.dropped += other.dropped;
        for (k, v) in &other.errors {
            *self.errors.entry(k.clone()).or_insert(0) += v;
        }
        self.frames += other.frames;
        self.inputs_sent += other.inputs_sent;
        self.inputs_acked += other.inputs_acked;
        self.bots_acked += other.bots_acked;
        self.inputs_unacked_dropped += other.inputs_unacked_dropped;
        for (k, v) in &other.rejections {
            *self.rejections.entry(k.clone()).or_insert(0) += v;
        }
        self.decode_errors += other.decode_errors;
        self.snapshot_interval_us.merge(&other.snapshot_interval_us)?;
        self.ack_latency_us.merge(&other.ack_latency_us)?;
        self.tick_rate_mhz.merge(&other.tick_rate_mhz)?;
        self.bytes_per_sec.merge(&other.bytes_per_sec)?;
        Ok(())
    }

    pub fn summary(&self) -> serde_json::Value {
        let ms = |h: &Histogram, q: f64| h.quantile(q).map(|v| v as f64 / 1000.0);
        serde_json::json!({
            "bots": self.bots,
            "connected": self.connected,
            "failed": self.failed,
            "dropped": self.dropped,
            "errors": self.errors,
            "frames": self.frames,
            "inputs_sent": self.inputs_sent,
            "inputs_acked": self.inputs_acked,
            "bots_acked": self.bots_acked,
            "inputs_unacked_dropped": self.inputs_unacked_dropped,
            "rejections": self.rejections,
            "decode_errors": self.decode_errors,
            "tick_rate_hz": {
                "p50": ms(&self.tick_rate_mhz, 0.5),
                "min": self.tick_rate_mhz.quantile(0.0).map(|v| v as f64 / 1000.0),
            },
            "snapshot_interval_ms": {
                "p50": ms(&self.snapshot_interval_us, 0.5),
                "p99": ms(&self.snapshot_interval_us, 0.99),
                "max": self.snapshot_interval_us.max as f64 / 1000.0,
                "jitter_p99_minus_p50": match (ms(&self.snapshot_interval_us, 0.99), ms(&self.snapshot_interval_us, 0.5)) {
                    (Some(a), Some(b)) => Some(a - b),
                    _ => None,
                },
            },
            "ack_latency_ms": {
                "p50": ms(&self.ack_latency_us, 0.5),
                "p99": ms(&self.ack_latency_us, 0.99),
                "max": self.ack_latency_us.max as f64 / 1000.0,
            },
            "bytes_per_sec_per_bot": {
                "p50": self.bytes_per_sec.quantile(0.5),
                "p99": self.bytes_per_sec.quantile(0.99),
            },
        })
    }

    pub fn print(&self) {
        let ms = |h: &Histogram, q: f64| {
            h.quantile(q)
                .map(|v| format!("{:.1} ms", v as f64 / 1000.0))
                .unwrap_or_else(|| "-".into())
        };
        let kb = |v: Option<u64>| {
            v.map(|b| format!("{:.1} KB/s", b as f64 / 1000.0))
                .unwrap_or_else(|| "-".into())
        };
        for run in &self.runs {
            println!(
                "Shard bench: {} bots from #{} for {:.0} s, {} inputs/s each, {}",
                run.bots, run.bot_offset, run.duration_secs, run.rate, run.url
            );
        }
        println!(
            "  connected          {} of {} ({} failed, {} dropped before the end)",
            self.connected, self.bots, self.failed, self.dropped
        );
        for (why, n) in &self.errors {
            println!("    {n} x {why}");
        }
        let hz = |v: Option<u64>| {
            v.map(|m| format!("{:.1} Hz", m as f64 / 1000.0))
                .unwrap_or_else(|| "-".into())
        };
        println!(
            "  tick rate seen     p50 {}   lowest bot {}",
            hz(self.tick_rate_mhz.quantile(0.5)),
            hz(self.tick_rate_mhz.quantile(0.0))
        );
        println!(
            "  snapshot interval  p50 {}   p99 {}   max {:.1} ms",
            ms(&self.snapshot_interval_us, 0.5),
            ms(&self.snapshot_interval_us, 0.99),
            self.snapshot_interval_us.max as f64 / 1000.0
        );
        println!(
            "  input to ack       p50 {}   p99 {}   ({} of {} inputs acked)",
            ms(&self.ack_latency_us, 0.5),
            ms(&self.ack_latency_us, 0.99),
            self.inputs_acked,
            self.inputs_sent
        );
        println!(
            "  bytes per bot      p50 {}   p99 {}",
            kb(self.bytes_per_sec.quantile(0.5)),
            kb(self.bytes_per_sec.quantile(0.99))
        );
        if self.rejections.is_empty() {
            println!("  rejections         none");
        } else {
            let list: Vec<String> = self
                .rejections
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect();
            println!("  rejections         {}", list.join(", "));
        }
        if self.decode_errors > 0 {
            println!("  decode errors      {}", self.decode_errors);
        }
    }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

fn run_shard(config: BenchConfig, json_mode: bool) -> ExitCode {
    // tokio-tungstenite uses rustls for wss://. The workspace builds rustls
    // with ring only; install it as the process provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            output::print_error(&format!("cannot start the async runtime: {e}"));
            return ExitCode::Error;
        }
    };
    if !json_mode {
        println!(
            "Running {} bots against {} for {:.0} s...",
            config.bots,
            config.url,
            config.duration.as_secs_f64()
        );
    }
    let report = runtime.block_on(run_bots(Arc::new(config.clone())));

    if let Some(path) = &config.report {
        let body = serde_json::to_string_pretty(&report).unwrap_or_default();
        if let Err(e) = std::fs::write(path, body) {
            output::print_error(&format!("cannot write {path}: {e}"));
            return ExitCode::Error;
        }
    }
    if json_mode {
        println!("{}", report.summary());
    } else {
        report.print();
    }
    if report.connected == 0 {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

async fn run_bots(config: Arc<BenchConfig>) -> Report {
    let report = Arc::new(Mutex::new(Report {
        runs: vec![RunInfo {
            url: config.url.clone(),
            bots: config.bots as u64,
            bot_offset: config.bot_offset as u64,
            duration_secs: config.duration.as_secs_f64(),
            rate: config.rate,
        }],
        bots: config.bots as u64,
        ..Report::default()
    }));
    let mut tasks = Vec::with_capacity(config.bots);
    for i in 0..config.bots {
        let bot = config.bot_offset + i;
        let delay = config.ramp.mul_f64(i as f64 / config.bots as f64);
        let (config, report) = (Arc::clone(&config), Arc::clone(&report));
        tasks.push(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let outcome = run_bot(&config, bot).await;
            let mut r = report.lock().unwrap();
            match outcome {
                Ok(stats) => {
                    r.connected += 1;
                    if stats.dropped {
                        r.dropped += 1;
                    }
                    stats.add_to(&mut r);
                }
                Err(why) => {
                    r.failed += 1;
                    *r.errors.entry(why).or_insert(0) += 1;
                }
            }
        }));
    }
    for t in tasks {
        if let Err(e) = t.await {
            let mut r = report.lock().unwrap();
            r.failed += 1;
            *r.errors.entry(format!("bot task failed: {e}")).or_insert(0) += 1;
        }
    }
    let report = report.lock().unwrap().clone();
    report
}

struct Join {
    shard: String,
    sid: String,
    ticket: String,
    token: Option<String>,
}

/// Join like a player: a guest session, then the app's join function.
fn join_with_function(config: &BenchConfig, function: &str) -> Result<Join, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let guest: serde_json::Value = agent
        .post(&format!("{}/api/auth/guest", config.url))
        .send_json(serde_json::json!({}))
        .map_err(|e| short_http_error("guest sign-in", e))?
        .into_json()
        .map_err(|e| format!("guest sign-in: bad JSON: {e}"))?;
    let token = guest["token"]
        .as_str()
        .ok_or("guest sign-in: no token in the response")?
        .to_string();
    let joined: serde_json::Value = agent
        .post(&format!("{}/api/fn/{function}", config.url))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({}))
        .map_err(|e| short_http_error(function, e))?
        .into_json()
        .map_err(|e| format!("{function}: bad JSON: {e}"))?;
    let field = |k: &str| {
        joined[k].as_str().map(str::to_string).ok_or_else(|| {
            format!(
                "{function} returned no {k} (it must return {{ shardId, subscriberId, ticket }})"
            )
        })
    };
    Ok(Join {
        shard: field("shardId")?,
        sid: field("subscriberId")?,
        ticket: field("ticket")?,
        token: Some(token),
    })
}

fn short_http_error(what: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let body: String = body.chars().take(160).collect();
            format!("{what}: HTTP {code} {body}")
        }
        ureq::Error::Transport(t) => format!("{what}: {}", t.kind()),
    }
}

/// What one bot measured.
#[derive(Default)]
struct BotStats {
    frames: u64,
    inputs_sent: u64,
    inputs_acked: u64,
    inputs_unacked_dropped: u64,
    rejections: BTreeMap<String, u64>,
    decode_errors: u64,
    snapshot_interval_us: Histogram,
    ack_latency_us: Histogram,
    ticks_seen: u64,
    bytes: u64,
    connected_for: Duration,
    dropped: bool,
}

impl BotStats {
    fn add_to(self, r: &mut Report) {
        r.frames += self.frames;
        r.inputs_sent += self.inputs_sent;
        r.inputs_acked += self.inputs_acked;
        r.inputs_unacked_dropped += self.inputs_unacked_dropped;
        if self.inputs_acked > 0 {
            r.bots_acked += 1;
        }
        for (k, v) in self.rejections {
            *r.rejections.entry(k).or_insert(0) += v;
        }
        r.decode_errors += self.decode_errors;
        // A bot's own counts cannot overflow the run's total.
        let _ = r.snapshot_interval_us.merge(&self.snapshot_interval_us);
        let _ = r.ack_latency_us.merge(&self.ack_latency_us);
        let secs = self.connected_for.as_secs_f64();
        if secs > 0.0 {
            r.tick_rate_mhz
                .record((self.ticks_seen as f64 / secs * 1000.0) as u64);
            r.bytes_per_sec.record((self.bytes as f64 / secs) as u64);
        }
    }
}

/// Inputs a bot keeps waiting for an ack. Past this the oldest is dropped
/// and counted, so a server that reads inputs and never acks them cannot
/// grow the bench's memory without bound.
const MAX_PENDING: usize = 10_000;

async fn run_bot(config: &BenchConfig, bot: usize) -> Result<BotStats, String> {
    let join = match &config.join {
        JoinMode::Function(f) => {
            let (c, f) = (config.clone(), f.clone());
            tokio::task::spawn_blocking(move || join_with_function(&c, &f))
                .await
                .map_err(|e| format!("join task failed: {e}"))??
        }
        JoinMode::Shard(shard) => {
            // The key comes from the same env files `pylon dev` loads.
            static ENV: std::sync::Once = std::sync::Once::new();
            ENV.call_once(|| {
                crate::commands::dev::load_env_files();
            });
            let sid = format!("{}{bot}", config.sid_prefix);
            let ticket = pylon_runtime::shard_tickets::mint(
                shard,
                &sid,
                None,
                serde_json::json!({ "bot": true }),
                Some(pylon_runtime::shard_tickets::MAX_TTL_SECS),
            );
            Join {
                shard: shard.clone(),
                sid,
                ticket,
                token: None,
            }
        }
    };

    let url = format!(
        "{}/shard?shard={}&sid={}&v=2",
        config.ws_base(),
        urlencode(&join.shard),
        urlencode(&join.sid)
    );
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("bad shard URL: {e}"))?;
    let mut protocols = vec![format!("ticket.{}", urlencode(&join.ticket))];
    if let Some(token) = &join.token {
        protocols.push(format!("bearer.{}", urlencode(token)));
    }
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        // No space after the comma: tungstenite's client splits the header on
        // "," without trimming and must find the server's echo in the list.
        HeaderValue::from_str(&protocols.join(","))
            .map_err(|e| format!("bad subprotocol header: {e}"))?,
    );
    let (ws, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "connect: timed out".to_string())?
    .map_err(|e| format!("connect: {}", short_ws_error(&e)))?;
    let (mut sink, mut stream) = ws.split();

    let started = Instant::now();
    let end = tokio::time::Instant::now() + config.duration;
    let mut stats = BotStats::default();
    let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(bot as u64);
    // Sent inputs not yet acked: (client_seq, sent at).
    let mut pending: VecDeque<(u64, Instant)> = VecDeque::new();
    let mut seq: u64 = 0;
    let mut codec: Option<u8> = None;
    let mut last_frame: Option<Instant> = None;
    let mut last_tick: u64 = 0;
    let mut input_timer = (config.rate > 0.0).then(|| {
        let mut t = tokio::time::interval(Duration::from_secs_f64(1.0 / config.rate));
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        t
    });

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(end) => break,
            _ = async {
                match input_timer.as_mut() {
                    Some(t) => { t.tick().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {
                let template = &config.inputs[(seq as usize) % config.inputs.len()];
                seq += 1;
                let input = expand_template(template, bot, &join.sid, &mut rng);
                let envelope = serde_json::json!({ "input": input, "client_seq": seq });
                // Binary MessagePack once the shard's codec is known to be
                // MessagePack; JSON text otherwise, which every shard takes.
                let msg = if codec == Some(wire::codec::MESSAGE_PACK) {
                    match rmp_serde::to_vec_named(&envelope) {
                        Ok(b) => Message::Binary(b),
                        Err(_) => Message::Text(envelope.to_string()),
                    }
                } else {
                    Message::Text(envelope.to_string())
                };
                // Timestamp before the send: time spent sending into a slow
                // connection is part of the latency.
                let sent_at = Instant::now();
                // A server that stops reading fills the socket; the send
                // must not outlive the run.
                match tokio::time::timeout_at(end, sink.send(msg)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        stats.dropped = true;
                        break;
                    }
                    Err(_) => break,
                }
                if pending.len() >= MAX_PENDING {
                    pending.pop_front();
                    stats.inputs_unacked_dropped += 1;
                }
                pending.push_back((seq, sent_at));
                stats.inputs_sent += 1;
            }
            next = stream.next() => {
                let bytes = match next {
                    Some(Ok(Message::Binary(b))) => b,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        stats.dropped = true;
                        break;
                    }
                    Some(Ok(_)) => continue,
                };
                let now = Instant::now();
                stats.bytes += bytes.len() as u64;
                let Some(frame) = parse_frame(&bytes) else {
                    stats.decode_errors += 1;
                    continue;
                };
                codec = Some(frame.codec);
                match frame.kind {
                    wire::kind::SNAPSHOT => {
                        stats.frames += 1;
                        if decode_payload::<serde::de::IgnoredAny>(frame.codec, frame.payload).is_none() {
                            stats.decode_errors += 1;
                        }
                        if let Some(prev) = last_frame {
                            stats.snapshot_interval_us.record(now.duration_since(prev).as_micros() as u64);
                        }
                        last_frame = Some(now);
                        if frame.tick > last_tick {
                            stats.ticks_seen += 1;
                            last_tick = frame.tick;
                        }
                    }
                    wire::kind::INPUT_REJECTED => {
                        match decode_payload::<InputRejection>(frame.codec, frame.payload) {
                            Some(r) => {
                                // A refused input may never be acked (refused
                                // before it was queued); stop waiting for it.
                                if let Some(seq) = r.client_seq {
                                    if let Some(i) = pending.iter().position(|(s, _)| *s == seq) {
                                        pending.remove(i);
                                    }
                                }
                                *stats.rejections.entry(r.code).or_insert(0) += 1;
                            }
                            None => stats.decode_errors += 1,
                        }
                    }
                    _ => stats.decode_errors += 1,
                }
                while let Some(&(s, sent)) = pending.front() {
                    if s > frame.ack {
                        break;
                    }
                    pending.pop_front();
                    stats.inputs_acked += 1;
                    stats.ack_latency_us.record(now.duration_since(sent).as_micros() as u64);
                }
            }
        }
    }
    stats.connected_for = started.elapsed();
    let _ = tokio::time::timeout(Duration::from_secs(2), sink.send(Message::Close(None))).await;
    Ok(stats)
}

struct ParsedFrame<'a> {
    kind: u8,
    codec: u8,
    tick: u64,
    ack: u64,
    payload: &'a [u8],
}

/// Split a version 2 frame (see `pylon_realtime::wire`).
fn parse_frame(bytes: &[u8]) -> Option<ParsedFrame<'_>> {
    if bytes.len() < wire::HEADER_LEN {
        return None;
    }
    let u64_at = |i: usize| u64::from_be_bytes(bytes[i..i + 8].try_into().unwrap());
    Some(ParsedFrame {
        kind: bytes[0],
        codec: bytes[1],
        tick: u64_at(2),
        ack: u64_at(10),
        payload: &bytes[wire::HEADER_LEN..],
    })
}

fn decode_payload<T: serde::de::DeserializeOwned>(codec: u8, payload: &[u8]) -> Option<T> {
    match codec {
        wire::codec::JSON => serde_json::from_slice(payload).ok(),
        wire::codec::MESSAGE_PACK => rmp_serde::from_slice(payload).ok(),
        // bincode and custom codecs: count the bytes, skip the decode.
        _ => None,
    }
}

fn short_ws_error(e: &tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error;
    match e {
        Error::Http(resp) => format!("HTTP {}", resp.status()),
        other => other.to_string(),
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// merge
// ---------------------------------------------------------------------------

fn run_merge(files: &[&str], json_mode: bool) -> ExitCode {
    let files: Vec<&str> = files.iter().copied().filter(|f| *f != "--json").collect();
    if files.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::Usage;
    }
    let mut total = Report::default();
    for f in &files {
        let parsed = std::fs::read_to_string(f)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Report>(&s).map_err(|e| e.to_string()));
        match parsed {
            Ok(r) => {
                for (name, h) in [
                    ("snapshot_interval_us", &r.snapshot_interval_us),
                    ("ack_latency_us", &r.ack_latency_us),
                    ("tick_rate_mhz", &r.tick_rate_mhz),
                    ("bytes_per_sec", &r.bytes_per_sec),
                ] {
                    if let Err(e) = h.validate() {
                        output::print_error(&format!("{f}: {name}: {e}"));
                        return ExitCode::Error;
                    }
                }
                if let Err(e) = total.merge(&r) {
                    output::print_error(&format!("{f}: {e}"));
                    return ExitCode::Error;
                }
            }
            Err(e) => {
                output::print_error(&format!("{f}: {e}"));
                return ExitCode::Error;
            }
        }
    }
    if json_mode {
        println!("{}", total.summary());
    } else {
        total.print();
    }
    ExitCode::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_quantiles_are_within_bucket_precision() {
        let mut h = Histogram::default();
        for v in 1..=10_000u64 {
            h.record(v);
        }
        for (q, want) in [(0.5, 5_000.0), (0.99, 9_900.0)] {
            let got = h.quantile(q).unwrap() as f64;
            assert!((got - want).abs() / want < 0.04, "q{q}: {got} vs {want}");
        }
        assert_eq!(h.quantile(1.0), Some(10_000));
        assert_eq!(h.max, 10_000);
    }

    #[test]
    fn small_values_are_exact_and_buckets_are_monotonic() {
        let mut last = 0;
        for v in 0..100_000u64 {
            let b = Histogram::bucket_of(v);
            assert!(b >= last);
            last = b;
            assert!(Histogram::value_of(b) >= v, "{v} -> bucket {b}");
        }
        let mut h = Histogram::default();
        h.record(3);
        assert_eq!(h.quantile(0.5), Some(3));
    }

    #[test]
    fn merged_histograms_equal_one_histogram_of_both() {
        let (mut a, mut b, mut both) = (
            Histogram::default(),
            Histogram::default(),
            Histogram::default(),
        );
        for v in 0..500u64 {
            a.record(v * 7);
            both.record(v * 7);
        }
        for v in 0..300u64 {
            b.record(v * 13 + 1);
            both.record(v * 13 + 1);
        }
        a.merge(&b).unwrap();
        assert_eq!(a, both);
    }

    #[test]
    fn min_is_exact_and_extreme_buckets_do_not_overflow() {
        let mut h = Histogram::default();
        h.record(19_900);
        h.record(20_100);
        assert_eq!(h.quantile(0.0), Some(19_900));
        let mut big = Histogram::default();
        big.record(u64::MAX);
        assert_eq!(big.quantile(0.5), Some(u64::MAX));
        assert!(Histogram::mid_of(Histogram::max_bucket()) > u64::MAX / 2);
        assert!(big.validate().is_ok());
    }

    #[test]
    fn merging_an_old_report_without_a_minimum_keeps_the_minimum_unknown() {
        let old = Histogram {
            buckets: [(Histogram::bucket_of(1), 1)].into_iter().collect(),
            count: 1,
            max: 1,
            min: None,
        };
        let mut new = Histogram::default();
        new.record(10);
        new.merge(&old).unwrap();
        assert_eq!(new.min, None);
        assert_eq!(new.quantile(0.0), Some(1));
        // An empty histogram is neutral.
        let mut empty = Histogram::default();
        empty.merge(&Histogram::default()).unwrap();
        let mut one = Histogram::default();
        one.record(5);
        empty.merge(&one).unwrap();
        assert_eq!(empty.min, Some(5));
    }

    #[test]
    fn a_merge_that_would_overflow_the_count_is_refused() {
        let big = |bucket: u32| Histogram {
            buckets: [(bucket, 10_000_000_000_000_000_000)].into_iter().collect(),
            count: 10_000_000_000_000_000_000,
            max: 2,
            min: Some(1),
        };
        let mut a = big(1);
        assert!(a.merge(&big(2)).is_err());
        assert_eq!(a, big(1));
    }

    #[test]
    fn imported_histograms_are_validated() {
        let bad_bucket = Histogram {
            buckets: [(u32::MAX, 1)].into_iter().collect(),
            count: 1,
            max: 1,
            min: Some(1),
        };
        assert!(bad_bucket.validate().is_err());
        let bad_count = Histogram {
            buckets: [(3, 2)].into_iter().collect(),
            count: 5,
            max: 3,
            min: None,
        };
        assert!(bad_count.validate().is_err());
    }

    #[test]
    fn out_of_range_arguments_are_refused() {
        let base = ["--shard", "s", "--rate", "0"];
        let with = |extra: &[&'static str]| {
            let mut v: Vec<&str> = base.to_vec();
            v.extend_from_slice(extra);
            BenchConfig::parse(&v)
        };
        assert!(with(&["--duration", "1e100"]).is_err());
        assert!(with(&["--ramp", "1e100"]).is_err());
        assert!(with(&["--bots", "0"]).is_err());
        let fast = BenchConfig::parse(&["--shard", "s", "--rate", "1e20", "--input", "1"]);
        assert!(fast.is_err());
    }

    #[test]
    fn templates_fill_placeholders() {
        let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(1);
        let t: serde_json::Value = serde_json::json!({
            "move_to": { "x": "$rand:0:800", "y": "$int:5:5" },
            "who": ["$bot", "$sid"],
            "keep": "text",
        });
        let v = expand_template(&t, 7, "bot-7", &mut rng);
        let x = v["move_to"]["x"].as_f64().unwrap();
        assert!((0.0..=800.0).contains(&x));
        assert_eq!(v["move_to"]["y"], 5);
        assert_eq!(v["who"], serde_json::json!([7, "bot-7"]));
        assert_eq!(v["keep"], "text");
    }

    #[test]
    fn config_needs_a_join_and_templates_for_inputs() {
        assert!(BenchConfig::parse(&["--bots", "3"])
            .unwrap_err()
            .contains("--join"));
        assert!(BenchConfig::parse(&["--join", "j"])
            .unwrap_err()
            .contains("--input"));
        let c = BenchConfig::parse(&[
            "--join",
            "joinArena",
            "--bots",
            "300",
            "--input",
            "\"join\"",
            "--url",
            "https://a.stack0.app/",
        ])
        .unwrap();
        assert_eq!(c.bots, 300);
        assert_eq!(c.join, JoinMode::Function("joinArena".into()));
        assert_eq!(c.ws_base(), "wss://a.stack0.app");
        let local = BenchConfig::parse(&["--shard", "s", "--rate", "0"]).unwrap();
        assert_eq!(local.ws_base(), "ws://localhost:4321");
    }

    #[test]
    fn frames_parse_like_the_wire_module_writes_them() {
        let f = wire::frame_v2(wire::kind::SNAPSHOT, wire::codec::JSON, 9, 4, b"{}");
        let p = parse_frame(&f).unwrap();
        assert_eq!(
            (p.kind, p.codec, p.tick, p.ack, p.payload),
            (1, 0, 9, 4, &b"{}"[..])
        );
        assert!(parse_frame(&f[..10]).is_none());
    }
}
