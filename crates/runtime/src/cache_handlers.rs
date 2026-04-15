//! Shared cache and pub/sub HTTP request handlers.
//!
//! These functions implement the cache command dispatch and pub/sub operations.
//! They are used by both the main server (`/api/cache`, `/api/pubsub/*`) and
//! the standalone cache server (`/cache`, `/pubsub/*`).

use agentdb_plugin::builtin::cache::CachePlugin;
use crate::pubsub::PubSubBroker;

// ---------------------------------------------------------------------------
// Cache command dispatch
// ---------------------------------------------------------------------------

/// Handle a `POST /cache` (or `POST /api/cache`) request body.
///
/// Parses the JSON body, extracts the `cmd` field, and dispatches to the
/// appropriate `CachePlugin` method. Returns `(http_status, json_body)`.
pub fn handle_cache_command(cache: &CachePlugin, body: &str) -> (u16, String) {
    let data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("Invalid JSON: {e}")}).to_string(),
            )
        }
    };

    let cmd = data
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_uppercase();
    let key = data.get("key").and_then(|v| v.as_str()).unwrap_or("");

    match cmd.as_str() {
        "SET" => {
            let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let ttl = data.get("ttl").and_then(|v| v.as_u64());
            cache.set(key, value, ttl);
            (200, serde_json::json!({"ok": true}).to_string())
        }
        "GET" => match cache.get(key) {
            Some(v) => (
                200,
                serde_json::json!({"ok": true, "result": v}).to_string(),
            ),
            None => (
                200,
                serde_json::json!({"ok": true, "result": null}).to_string(),
            ),
        },
        "DEL" => {
            let deleted = cache.del(key);
            (
                200,
                serde_json::json!({"ok": true, "result": deleted}).to_string(),
            )
        }
        "EXISTS" => {
            let exists = cache.exists(key);
            (
                200,
                serde_json::json!({"ok": true, "result": exists}).to_string(),
            )
        }
        "INCR" => match cache.incr(key) {
            Ok(n) => (
                200,
                serde_json::json!({"ok": true, "result": n}).to_string(),
            ),
            Err(e) => (
                400,
                serde_json::json!({"ok": false, "error": e}).to_string(),
            ),
        },
        "DECR" => match cache.decr(key) {
            Ok(n) => (
                200,
                serde_json::json!({"ok": true, "result": n}).to_string(),
            ),
            Err(e) => (
                400,
                serde_json::json!({"ok": false, "error": e}).to_string(),
            ),
        },
        "INCRBY" => {
            let amount = data.get("amount").and_then(|v| v.as_i64()).unwrap_or(1);
            match cache.incrby(key, amount) {
                Ok(n) => (
                    200,
                    serde_json::json!({"ok": true, "result": n}).to_string(),
                ),
                Err(e) => (
                    400,
                    serde_json::json!({"ok": false, "error": e}).to_string(),
                ),
            }
        }
        "SETNX" => {
            let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let ttl = data.get("ttl").and_then(|v| v.as_u64());
            let was_set = cache.setnx(key, value, ttl);
            (
                200,
                serde_json::json!({"ok": true, "result": was_set}).to_string(),
            )
        }
        "GETSET" => {
            let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let old = cache.getset(key, value);
            (
                200,
                serde_json::json!({"ok": true, "result": old}).to_string(),
            )
        }
        "MGET" => {
            let keys_arr: Vec<&str> = data
                .get("keys")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let results = cache.mget(&keys_arr);
            (
                200,
                serde_json::json!({"ok": true, "result": results}).to_string(),
            )
        }
        "MSET" => {
            let pairs_val = data.get("pairs").and_then(|v| v.as_object());
            if let Some(obj) = pairs_val {
                let pairs: Vec<(&str, &str)> = obj
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s)))
                    .collect();
                cache.mset(&pairs);
                (200, serde_json::json!({"ok": true}).to_string())
            } else {
                (
                    400,
                    serde_json::json!({"ok": false, "error": "pairs object required"}).to_string(),
                )
            }
        }
        "LPUSH" => {
            let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let len = cache.lpush(key, value);
            (
                200,
                serde_json::json!({"ok": true, "result": len}).to_string(),
            )
        }
        "RPUSH" => {
            let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let len = cache.rpush(key, value);
            (
                200,
                serde_json::json!({"ok": true, "result": len}).to_string(),
            )
        }
        "LPOP" => {
            let val = cache.lpop(key);
            (
                200,
                serde_json::json!({"ok": true, "result": val}).to_string(),
            )
        }
        "RPOP" => {
            let val = cache.rpop(key);
            (
                200,
                serde_json::json!({"ok": true, "result": val}).to_string(),
            )
        }
        "LRANGE" => {
            let start = data.get("start").and_then(|v| v.as_i64()).unwrap_or(0);
            let stop = data.get("stop").and_then(|v| v.as_i64()).unwrap_or(-1);
            let items = cache.lrange(key, start, stop);
            (
                200,
                serde_json::json!({"ok": true, "result": items}).to_string(),
            )
        }
        "LLEN" => {
            let len = cache.llen(key);
            (
                200,
                serde_json::json!({"ok": true, "result": len}).to_string(),
            )
        }
        "SADD" => {
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let added = cache.sadd(key, member);
            (
                200,
                serde_json::json!({"ok": true, "result": added}).to_string(),
            )
        }
        "SREM" => {
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let removed = cache.srem(key, member);
            (
                200,
                serde_json::json!({"ok": true, "result": removed}).to_string(),
            )
        }
        "SMEMBERS" => {
            let members = cache.smembers(key);
            (
                200,
                serde_json::json!({"ok": true, "result": members}).to_string(),
            )
        }
        "SISMEMBER" => {
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let is_member = cache.sismember(key, member);
            (
                200,
                serde_json::json!({"ok": true, "result": is_member}).to_string(),
            )
        }
        "SCARD" => {
            let count = cache.scard(key);
            (
                200,
                serde_json::json!({"ok": true, "result": count}).to_string(),
            )
        }
        "SINTER" => {
            let key2 = data.get("key2").and_then(|v| v.as_str()).unwrap_or("");
            let inter = cache.sinter(key, key2);
            (
                200,
                serde_json::json!({"ok": true, "result": inter}).to_string(),
            )
        }
        "SUNION" => {
            let key2 = data.get("key2").and_then(|v| v.as_str()).unwrap_or("");
            let union_result = cache.sunion(key, key2);
            (
                200,
                serde_json::json!({"ok": true, "result": union_result}).to_string(),
            )
        }
        "HSET" => {
            let field = data.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
            cache.hset(key, field, value);
            (200, serde_json::json!({"ok": true}).to_string())
        }
        "HGET" => {
            let field = data.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let val = cache.hget(key, field);
            (
                200,
                serde_json::json!({"ok": true, "result": val}).to_string(),
            )
        }
        "HDEL" => {
            let field = data.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let deleted = cache.hdel(key, field);
            (
                200,
                serde_json::json!({"ok": true, "result": deleted}).to_string(),
            )
        }
        "HGETALL" => {
            let all = cache.hgetall(key);
            (
                200,
                serde_json::json!({"ok": true, "result": all}).to_string(),
            )
        }
        "HEXISTS" => {
            let field = data.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let exists = cache.hexists(key, field);
            (
                200,
                serde_json::json!({"ok": true, "result": exists}).to_string(),
            )
        }
        "HLEN" => {
            let len = cache.hlen(key);
            (
                200,
                serde_json::json!({"ok": true, "result": len}).to_string(),
            )
        }
        "HKEYS" => {
            let keys = cache.hkeys(key);
            (
                200,
                serde_json::json!({"ok": true, "result": keys}).to_string(),
            )
        }
        "HINCRBY" => {
            let field = data.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let amount = data.get("amount").and_then(|v| v.as_i64()).unwrap_or(1);
            match cache.hincrby(key, field, amount) {
                Ok(n) => (
                    200,
                    serde_json::json!({"ok": true, "result": n}).to_string(),
                ),
                Err(e) => (
                    400,
                    serde_json::json!({"ok": false, "error": e}).to_string(),
                ),
            }
        }
        "ZADD" => {
            let score = data.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            cache.zadd(key, score, member);
            (200, serde_json::json!({"ok": true}).to_string())
        }
        "ZREM" => {
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let removed = cache.zrem(key, member);
            (
                200,
                serde_json::json!({"ok": true, "result": removed}).to_string(),
            )
        }
        "ZSCORE" => {
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let score = cache.zscore(key, member);
            (
                200,
                serde_json::json!({"ok": true, "result": score}).to_string(),
            )
        }
        "ZRANK" => {
            let member = data.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let rank = cache.zrank(key, member);
            (
                200,
                serde_json::json!({"ok": true, "result": rank}).to_string(),
            )
        }
        "ZRANGE" => {
            let start = data.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let stop = data.get("stop").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let members = cache.zrange(key, start, stop);
            let result: Vec<serde_json::Value> = members
                .iter()
                .map(|(m, s)| serde_json::json!({"member": m, "score": s}))
                .collect();
            (
                200,
                serde_json::json!({"ok": true, "result": result}).to_string(),
            )
        }
        "ZCARD" => {
            let count = cache.zcard(key);
            (
                200,
                serde_json::json!({"ok": true, "result": count}).to_string(),
            )
        }
        "KEYS" => {
            let pattern = data
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            let keys = cache.keys(pattern);
            (
                200,
                serde_json::json!({"ok": true, "result": keys}).to_string(),
            )
        }
        "TTL" => {
            let ttl = cache.ttl(key);
            (
                200,
                serde_json::json!({"ok": true, "result": ttl}).to_string(),
            )
        }
        "EXPIRE" => {
            let seconds = data.get("seconds").and_then(|v| v.as_u64()).unwrap_or(0);
            let ok = cache.expire(key, seconds);
            (
                200,
                serde_json::json!({"ok": true, "result": ok}).to_string(),
            )
        }
        "PERSIST" => {
            let ok = cache.persist(key);
            (
                200,
                serde_json::json!({"ok": true, "result": ok}).to_string(),
            )
        }
        "TYPE" => {
            let t = cache.key_type(key);
            (
                200,
                serde_json::json!({"ok": true, "result": t}).to_string(),
            )
        }
        "DBSIZE" => {
            let size = cache.dbsize();
            (
                200,
                serde_json::json!({"ok": true, "result": size}).to_string(),
            )
        }
        "FLUSHALL" => {
            cache.flushall();
            (200, serde_json::json!({"ok": true}).to_string())
        }
        "INFO" => {
            let info = cache.info();
            (
                200,
                serde_json::json!({"ok": true, "result": info}).to_string(),
            )
        }
        "CLEANUP" => {
            let removed = cache.cleanup_expired();
            (
                200,
                serde_json::json!({"ok": true, "result": removed}).to_string(),
            )
        }
        _ => (
            400,
            serde_json::json!({"ok": false, "error": format!("Unknown cache command: {cmd}")})
                .to_string(),
        ),
    }
}

/// Handle a `GET /cache/:key` shorthand request.
pub fn handle_cache_get(cache: &CachePlugin, key: &str) -> (u16, String) {
    match cache.get(key) {
        Some(v) => (
            200,
            serde_json::json!({"ok": true, "result": v}).to_string(),
        ),
        None => (
            404,
            serde_json::json!({"ok": false, "error": "key not found"}).to_string(),
        ),
    }
}

/// Handle a `DELETE /cache/:key` shorthand request.
pub fn handle_cache_delete(cache: &CachePlugin, key: &str) -> (u16, String) {
    let deleted = cache.del(key);
    (
        if deleted { 200 } else { 404 },
        serde_json::json!({"ok": deleted}).to_string(),
    )
}

// ---------------------------------------------------------------------------
// Pub/Sub handlers
// ---------------------------------------------------------------------------

/// Handle a `POST /pubsub/publish` request.
pub fn handle_pubsub_publish(pubsub: &PubSubBroker, body: &str) -> (u16, String) {
    let data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("Invalid JSON: {e}")}).to_string(),
            )
        }
    };

    let channel = match data.get("channel").and_then(|v| v.as_str()) {
        Some(ch) => ch,
        None => {
            return (
                400,
                serde_json::json!({"error": {"code": "MISSING_CHANNEL", "message": "channel is required"}}).to_string(),
            )
        }
    };

    let message = match data.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => {
            return (
                400,
                serde_json::json!({"error": {"code": "MISSING_MESSAGE", "message": "message is required"}}).to_string(),
            )
        }
    };

    let subscribers = pubsub.publish(channel, message);
    (
        200,
        serde_json::json!({"ok": true, "subscribers": subscribers}).to_string(),
    )
}

/// Handle a `GET /pubsub/channels` request.
pub fn handle_pubsub_channels(pubsub: &PubSubBroker) -> (u16, String) {
    let channels = pubsub.channels();
    let result: Vec<serde_json::Value> = channels
        .iter()
        .map(|(ch, count)| serde_json::json!({"channel": ch, "subscribers": count}))
        .collect();
    (
        200,
        serde_json::json!({"ok": true, "result": result}).to_string(),
    )
}

/// Handle a `GET /pubsub/history/:channel` request.
///
/// The `url` parameter is the full URL path (used to parse `?limit=N`).
pub fn handle_pubsub_history(
    pubsub: &PubSubBroker,
    channel: &str,
    url: &str,
) -> (u16, String) {
    let limit: usize = url
        .split("limit=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(1000);
    let messages = pubsub.history(channel, limit);
    (
        200,
        serde_json::json!({"ok": true, "result": messages}).to_string(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cache() -> CachePlugin {
        CachePlugin::new(1000)
    }

    fn make_pubsub() -> PubSubBroker {
        PubSubBroker::new(100)
    }

    #[test]
    fn cache_set_and_get() {
        let cache = make_cache();
        let (status, _) = handle_cache_command(
            &cache,
            r#"{"cmd": "SET", "key": "hello", "value": "world"}"#,
        );
        assert_eq!(status, 200);

        let (status, body) =
            handle_cache_command(&cache, r#"{"cmd": "GET", "key": "hello"}"#);
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], "world");
    }

    #[test]
    fn cache_get_shorthand() {
        let cache = make_cache();
        cache.set("mykey", "myval", None);
        let (status, body) = handle_cache_get(&cache, "mykey");
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], "myval");
    }

    #[test]
    fn cache_get_shorthand_missing() {
        let cache = make_cache();
        let (status, _) = handle_cache_get(&cache, "nokey");
        assert_eq!(status, 404);
    }

    #[test]
    fn cache_delete_shorthand() {
        let cache = make_cache();
        cache.set("k", "v", None);
        let (status, _) = handle_cache_delete(&cache, "k");
        assert_eq!(status, 200);
        let (status, _) = handle_cache_delete(&cache, "k");
        assert_eq!(status, 404);
    }

    #[test]
    fn cache_invalid_json() {
        let cache = make_cache();
        let (status, body) = handle_cache_command(&cache, "not json");
        assert_eq!(status, 400);
        assert!(body.contains("Invalid JSON"));
    }

    #[test]
    fn cache_unknown_command() {
        let cache = make_cache();
        let (status, body) =
            handle_cache_command(&cache, r#"{"cmd": "NOTACMD", "key": "k"}"#);
        assert_eq!(status, 400);
        assert!(body.contains("Unknown cache command"));
    }

    #[test]
    fn pubsub_publish_and_channels() {
        let pubsub = make_pubsub();
        let (status, body) = handle_pubsub_publish(
            &pubsub,
            r#"{"channel": "chat", "message": "hello"}"#,
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn pubsub_publish_missing_channel() {
        let pubsub = make_pubsub();
        let (status, _) = handle_pubsub_publish(&pubsub, r#"{"message": "hello"}"#);
        assert_eq!(status, 400);
    }

    #[test]
    fn pubsub_publish_missing_message() {
        let pubsub = make_pubsub();
        let (status, _) = handle_pubsub_publish(&pubsub, r#"{"channel": "ch"}"#);
        assert_eq!(status, 400);
    }

    #[test]
    fn pubsub_history() {
        let pubsub = make_pubsub();
        pubsub.publish("news", "headline 1");
        pubsub.publish("news", "headline 2");
        let (status, body) =
            handle_pubsub_history(&pubsub, "news", "/pubsub/history/news?limit=10");
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn pubsub_channels_list() {
        let pubsub = make_pubsub();
        let (status, body) = handle_pubsub_channels(&pubsub);
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn cache_incr_decr() {
        let cache = make_cache();
        let (status, body) =
            handle_cache_command(&cache, r#"{"cmd": "INCR", "key": "counter"}"#);
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], 1);

        let (_, body) =
            handle_cache_command(&cache, r#"{"cmd": "DECR", "key": "counter"}"#);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], 0);
    }

    #[test]
    fn cache_dbsize_and_flushall() {
        let cache = make_cache();
        cache.set("a", "1", None);
        cache.set("b", "2", None);
        let (_, body) = handle_cache_command(&cache, r#"{"cmd": "DBSIZE"}"#);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], 2);

        let (status, _) = handle_cache_command(&cache, r#"{"cmd": "FLUSHALL"}"#);
        assert_eq!(status, 200);
        assert_eq!(cache.dbsize(), 0);
    }
}
