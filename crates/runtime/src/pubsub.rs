use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Message type
// ---------------------------------------------------------------------------

/// A message published to a channel.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PubSubMessage {
    pub channel: String,
    pub message: String,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Subscriber callback
// ---------------------------------------------------------------------------

type Callback = Box<dyn Fn(&PubSubMessage) + Send + Sync>;

// ---------------------------------------------------------------------------
// PubSubBroker
// ---------------------------------------------------------------------------

/// In-memory pub/sub broker with channel-based messaging, history retention,
/// and glob-pattern subscriptions.
pub struct PubSubBroker {
    /// channel -> list of (subscriber_id, callback)
    subscriptions: Mutex<HashMap<String, Vec<(u64, Callback)>>>,
    next_id: Mutex<u64>,
    /// Recent messages per channel for late joiners.
    history: Mutex<HashMap<String, Vec<PubSubMessage>>>,
    max_history: usize,
}

impl PubSubBroker {
    /// Create a new broker that retains up to `max_history_per_channel`
    /// messages per channel.
    pub fn new(max_history_per_channel: usize) -> Self {
        Self {
            subscriptions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
            history: Mutex::new(HashMap::new()),
            max_history: max_history_per_channel,
        }
    }

    /// Publish a message to a channel. Returns the number of subscribers
    /// that were notified.
    pub fn publish(&self, channel: &str, message: &str) -> usize {
        let msg = PubSubMessage {
            channel: channel.to_string(),
            message: message.to_string(),
            timestamp: now_iso(),
        };

        // Save to history.
        {
            let mut history = self.history.lock().unwrap();
            let channel_history = history.entry(channel.to_string()).or_default();
            channel_history.push(msg.clone());
            if channel_history.len() > self.max_history {
                channel_history.remove(0);
            }
        }

        // Notify subscribers.
        let subs = self.subscriptions.lock().unwrap();
        if let Some(subscribers) = subs.get(channel) {
            for (_, callback) in subscribers {
                callback(&msg);
            }
            subscribers.len()
        } else {
            0
        }
    }

    /// Subscribe to a channel. Returns a subscription ID that can be used
    /// to unsubscribe later.
    pub fn subscribe(&self, channel: &str, callback: Callback) -> u64 {
        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };
        let mut subs = self.subscriptions.lock().unwrap();
        subs.entry(channel.to_string())
            .or_default()
            .push((id, callback));
        id
    }

    /// Unsubscribe from a channel by subscription ID. Returns true if the
    /// subscription was found and removed.
    pub fn unsubscribe(&self, channel: &str, sub_id: u64) -> bool {
        let mut subs = self.subscriptions.lock().unwrap();
        if let Some(subscribers) = subs.get_mut(channel) {
            let before = subscribers.len();
            subscribers.retain(|(id, _)| *id != sub_id);
            let removed = subscribers.len() < before;
            // Clean up empty channel entries.
            if subscribers.is_empty() {
                subs.remove(channel);
            }
            removed
        } else {
            false
        }
    }

    /// Get recent message history for a channel, up to `limit` messages.
    /// Returns messages in chronological order (oldest first).
    pub fn history(&self, channel: &str, limit: usize) -> Vec<PubSubMessage> {
        let history = self.history.lock().unwrap();
        match history.get(channel) {
            Some(msgs) => {
                let start = msgs.len().saturating_sub(limit);
                msgs[start..].to_vec()
            }
            None => vec![],
        }
    }

    /// List all channels that have at least one subscriber, along with their
    /// subscriber counts.
    pub fn channels(&self) -> Vec<(String, usize)> {
        let subs = self.subscriptions.lock().unwrap();
        let mut result: Vec<(String, usize)> = subs
            .iter()
            .map(|(ch, s)| (ch.clone(), s.len()))
            .collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    /// Get the number of subscribers for a specific channel.
    pub fn subscriber_count(&self, channel: &str) -> usize {
        let subs = self.subscriptions.lock().unwrap();
        subs.get(channel).map(|s| s.len()).unwrap_or(0)
    }

    /// Pattern-subscribe: subscribe to all existing channels whose names
    /// match a glob pattern. Returns the subscription IDs created (one per
    /// matched channel).
    ///
    /// Note: this is a snapshot-based pattern subscribe. Channels created
    /// after the call will not be matched automatically.
    pub fn psubscribe(&self, pattern: &str, callback: Callback) -> Vec<u64> {
        // Collect matching channel names first (to avoid holding both locks).
        let matching: Vec<String> = {
            let subs = self.subscriptions.lock().unwrap();
            subs.keys()
                .filter(|ch| glob_match(pattern, ch))
                .cloned()
                .collect()
        };

        // Also check history for channels that have messages but no current
        // subscribers.
        let history_channels: Vec<String> = {
            let history = self.history.lock().unwrap();
            history
                .keys()
                .filter(|ch| glob_match(pattern, ch) && !matching.contains(ch))
                .cloned()
                .collect()
        };

        let all_channels: Vec<String> = matching
            .into_iter()
            .chain(history_channels)
            .collect();

        // We need to create a shared callback that can be used across
        // multiple subscriptions. We wrap it in an Arc.
        let shared_cb = std::sync::Arc::new(callback);
        let mut ids = Vec::new();
        for ch in &all_channels {
            let cb = std::sync::Arc::clone(&shared_cb);
            let id = self.subscribe(ch, Box::new(move |msg| cb(msg)));
            ids.push(id);
        }
        ids
    }

    /// List all channels that have history entries (regardless of whether
    /// they have active subscribers).
    pub fn channels_with_history(&self) -> Vec<String> {
        let history = self.history.lock().unwrap();
        let mut channels: Vec<String> = history.keys().cloned().collect();
        channels.sort();
        channels
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the current UTC time as an ISO 8601 string.
fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple epoch-to-ISO conversion without the chrono crate.
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = is_leap(y);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i;
            break;
        }
        remaining -= md;
    }
    let d = remaining + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Simple glob matching supporting `*` (any sequence) and `?` (single char).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_inner(&pat, &txt)
}

fn glob_inner(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    match pat[0] {
        '*' => {
            for i in 0..=txt.len() {
                if glob_inner(&pat[1..], &txt[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => {
            if txt.is_empty() {
                false
            } else {
                glob_inner(&pat[1..], &txt[1..])
            }
        }
        c => {
            if txt.is_empty() || txt[0] != c {
                false
            } else {
                glob_inner(&pat[1..], &txt[1..])
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn publish_and_subscribe() {
        let broker = PubSubBroker::new(10);
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        broker.subscribe("chat", Box::new(move |_msg| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        let notified = broker.publish("chat", "hello");
        assert_eq!(notified, 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn publish_to_empty_channel() {
        let broker = PubSubBroker::new(10);
        let notified = broker.publish("empty", "no one listening");
        assert_eq!(notified, 0);
    }

    #[test]
    fn multiple_subscribers() {
        let broker = PubSubBroker::new(10);
        let count = Arc::new(AtomicUsize::new(0));
        for _ in 0..5 {
            let c = Arc::clone(&count);
            broker.subscribe("events", Box::new(move |_msg| {
                c.fetch_add(1, Ordering::SeqCst);
            }));
        }
        let notified = broker.publish("events", "boom");
        assert_eq!(notified, 5);
        assert_eq!(count.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn unsubscribe() {
        let broker = PubSubBroker::new(10);
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let id = broker.subscribe("ch", Box::new(move |_msg| {
            c.fetch_add(1, Ordering::SeqCst);
        }));

        broker.publish("ch", "first");
        assert_eq!(count.load(Ordering::SeqCst), 1);

        assert!(broker.unsubscribe("ch", id));
        broker.publish("ch", "second");
        // Count should still be 1 since we unsubscribed.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unsubscribe_nonexistent() {
        let broker = PubSubBroker::new(10);
        assert!(!broker.unsubscribe("nope", 999));
    }

    #[test]
    fn history_basic() {
        let broker = PubSubBroker::new(10);
        broker.publish("news", "headline 1");
        broker.publish("news", "headline 2");
        broker.publish("news", "headline 3");

        let msgs = broker.history("news", 10);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].message, "headline 1");
        assert_eq!(msgs[2].message, "headline 3");
    }

    #[test]
    fn history_limit() {
        let broker = PubSubBroker::new(10);
        for i in 0..10 {
            broker.publish("ch", &format!("msg {i}"));
        }
        let msgs = broker.history("ch", 3);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].message, "msg 7");
        assert_eq!(msgs[2].message, "msg 9");
    }

    #[test]
    fn history_eviction() {
        let broker = PubSubBroker::new(3);
        broker.publish("ch", "a");
        broker.publish("ch", "b");
        broker.publish("ch", "c");
        broker.publish("ch", "d");

        let msgs = broker.history("ch", 10);
        assert_eq!(msgs.len(), 3);
        // "a" should have been evicted.
        assert_eq!(msgs[0].message, "b");
    }

    #[test]
    fn history_empty_channel() {
        let broker = PubSubBroker::new(10);
        let msgs = broker.history("nonexistent", 10);
        assert!(msgs.is_empty());
    }

    #[test]
    fn channels_list() {
        let broker = PubSubBroker::new(10);
        broker.subscribe("alpha", Box::new(|_| {}));
        broker.subscribe("alpha", Box::new(|_| {}));
        broker.subscribe("beta", Box::new(|_| {}));

        let channels = broker.channels();
        assert_eq!(channels.len(), 2);
        // Sorted alphabetically.
        assert_eq!(channels[0].0, "alpha");
        assert_eq!(channels[0].1, 2);
        assert_eq!(channels[1].0, "beta");
        assert_eq!(channels[1].1, 1);
    }

    #[test]
    fn subscriber_count() {
        let broker = PubSubBroker::new(10);
        assert_eq!(broker.subscriber_count("ch"), 0);
        broker.subscribe("ch", Box::new(|_| {}));
        broker.subscribe("ch", Box::new(|_| {}));
        assert_eq!(broker.subscriber_count("ch"), 2);
    }

    #[test]
    fn pattern_subscribe() {
        let broker = PubSubBroker::new(10);
        // Create some channels via publish (so they appear in history).
        broker.publish("user:1", "event");
        broker.publish("user:2", "event");
        broker.publish("system:1", "event");

        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let ids = broker.psubscribe("user:*", Box::new(move |_msg| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(ids.len(), 2); // user:1 and user:2

        broker.publish("user:1", "hello");
        broker.publish("user:2", "world");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn message_contains_metadata() {
        let broker = PubSubBroker::new(10);
        let received = Arc::new(Mutex::new(None::<PubSubMessage>));
        let r = Arc::clone(&received);
        broker.subscribe("meta", Box::new(move |msg| {
            *r.lock().unwrap() = Some(msg.clone());
        }));
        broker.publish("meta", "payload");

        let msg = received.lock().unwrap().clone().unwrap();
        assert_eq!(msg.channel, "meta");
        assert_eq!(msg.message, "payload");
        assert!(!msg.timestamp.is_empty());
        // Timestamp should look like ISO 8601.
        assert!(msg.timestamp.contains('T'));
        assert!(msg.timestamp.ends_with('Z'));
    }

    #[test]
    fn glob_match_works() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("user:*", "user:123"));
        assert!(!glob_match("user:*", "session:1"));
        assert!(glob_match("u?er:*", "user:1"));
        assert!(!glob_match("u?er:*", "uuser:1"));
        assert!(glob_match("*:*", "a:b"));
    }

    #[test]
    fn channels_with_history_list() {
        let broker = PubSubBroker::new(10);
        broker.publish("alpha", "msg");
        broker.publish("beta", "msg");
        let channels = broker.channels_with_history();
        assert_eq!(channels.len(), 2);
        assert!(channels.contains(&"alpha".to_string()));
        assert!(channels.contains(&"beta".to_string()));
    }
}
