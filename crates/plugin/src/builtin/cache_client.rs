//! Remote cache client.
//!
//! Connects to a standalone pylon cache server over HTTP. Provides the
//! same logical API as [`CachePlugin`](super::cache::CachePlugin) so callers
//! can swap between embedded and remote cache without changing application
//! logic.
//!
//! # Example
//!
//! ```rust,ignore
//! use pylon_plugin::builtin::cache_client::RemoteCacheClient;
//!
//! let client = RemoteCacheClient::new("http://localhost:6380");
//! client.set("greeting", "hello", None).unwrap();
//! assert_eq!(client.get("greeting").unwrap(), Some("hello".to_string()));
//! ```

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// A client that connects to a remote pylon cache server.
///
/// Uses raw HTTP/1.1 over TCP to avoid pulling in heavy HTTP client
/// dependencies. Each request opens a new connection (`Connection: close`).
/// This keeps the client simple and dependency-free.
pub struct RemoteCacheClient {
    /// Parsed host:port string (no scheme, no trailing path).
    host_port: String,
}

impl RemoteCacheClient {
    /// Create a new client pointing at the given base URL.
    ///
    /// The URL should be of the form `http://host:port`. Any trailing slash
    /// or path component is stripped.
    pub fn new(base_url: &str) -> Self {
        let stripped = base_url.trim_end_matches('/');
        let host_port = stripped
            .strip_prefix("http://")
            .unwrap_or(stripped)
            .split('/')
            .next()
            .unwrap_or(stripped)
            .to_string();
        Self { host_port }
    }

    /// Send a POST request to `path` with a JSON body.
    /// Returns the parsed JSON response body.
    fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        let payload = body.to_string();
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            path, self.host_port, payload.len(), payload
        );

        let mut stream = TcpStream::connect(&self.host_port)
            .map_err(|e| format!("Cache connection to {} failed: {e}", self.host_port))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        // Shut down the write half so the server knows we are done sending.
        stream.shutdown(std::net::Shutdown::Write).ok();

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|e| format!("Read failed: {e}"))?;

        // Parse HTTP response -- body comes after the first blank line.
        let body_str = response.split("\r\n\r\n").nth(1).unwrap_or("{}");
        serde_json::from_str(body_str).map_err(|e| format!("Parse failed: {e}"))
    }

    /// Send a GET request to `path`.
    fn get(&self, path: &str) -> Result<serde_json::Value, String> {
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            path, self.host_port
        );

        let mut stream = TcpStream::connect(&self.host_port)
            .map_err(|e| format!("Cache connection to {} failed: {e}", self.host_port))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;
        stream.shutdown(std::net::Shutdown::Write).ok();

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|e| format!("Read failed: {e}"))?;

        let body_str = response.split("\r\n\r\n").nth(1).unwrap_or("{}");
        serde_json::from_str(body_str).map_err(|e| format!("Parse failed: {e}"))
    }

    /// Execute a cache command via `POST /cache`.
    fn execute(&self, cmd: serde_json::Value) -> Result<serde_json::Value, String> {
        self.post("/cache", &cmd)
    }

    // -----------------------------------------------------------------------
    // String operations
    // -----------------------------------------------------------------------

    /// SET key value [EX seconds]
    pub fn set(&self, key: &str, value: &str, ttl: Option<u64>) -> Result<(), String> {
        let mut cmd = serde_json::json!({"cmd": "SET", "key": key, "value": value});
        if let Some(t) = ttl {
            cmd["ttl"] = serde_json::json!(t);
        }
        self.execute(cmd)?;
        Ok(())
    }

    /// GET key
    pub fn get_key(&self, key: &str) -> Result<Option<String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "GET", "key": key}))?;
        Ok(result.get("result").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        }))
    }

    /// DEL key
    pub fn del(&self, key: &str) -> Result<bool, String> {
        let result = self.execute(serde_json::json!({"cmd": "DEL", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// EXISTS key
    pub fn exists(&self, key: &str) -> Result<bool, String> {
        let result = self.execute(serde_json::json!({"cmd": "EXISTS", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// INCR key
    pub fn incr(&self, key: &str) -> Result<i64, String> {
        let result = self.execute(serde_json::json!({"cmd": "INCR", "key": key}))?;
        result
            .get("result")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("INCR failed")
                    .to_string()
            })
    }

    /// DECR key
    pub fn decr(&self, key: &str) -> Result<i64, String> {
        let result = self.execute(serde_json::json!({"cmd": "DECR", "key": key}))?;
        result
            .get("result")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("DECR failed")
                    .to_string()
            })
    }

    /// INCRBY key amount
    pub fn incrby(&self, key: &str, amount: i64) -> Result<i64, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "INCRBY", "key": key, "amount": amount}))?;
        result
            .get("result")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("INCRBY failed")
                    .to_string()
            })
    }

    /// SETNX key value [EX seconds]
    pub fn setnx(&self, key: &str, value: &str, ttl: Option<u64>) -> Result<bool, String> {
        let mut cmd = serde_json::json!({"cmd": "SETNX", "key": key, "value": value});
        if let Some(t) = ttl {
            cmd["ttl"] = serde_json::json!(t);
        }
        let result = self.execute(cmd)?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// GETSET key value
    pub fn getset(&self, key: &str, value: &str) -> Result<Option<String>, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "GETSET", "key": key, "value": value}))?;
        Ok(result.get("result").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        }))
    }

    // -----------------------------------------------------------------------
    // List operations
    // -----------------------------------------------------------------------

    /// LPUSH key value
    pub fn lpush(&self, key: &str, value: &str) -> Result<usize, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "LPUSH", "key": key, "value": value}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "LPUSH failed".into())
    }

    /// RPUSH key value
    pub fn rpush(&self, key: &str, value: &str) -> Result<usize, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "RPUSH", "key": key, "value": value}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "RPUSH failed".into())
    }

    /// LPOP key
    pub fn lpop(&self, key: &str) -> Result<Option<String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "LPOP", "key": key}))?;
        Ok(result.get("result").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        }))
    }

    /// RPOP key
    pub fn rpop(&self, key: &str) -> Result<Option<String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "RPOP", "key": key}))?;
        Ok(result.get("result").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        }))
    }

    /// LRANGE key start stop
    pub fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<String>, String> {
        let result = self.execute(
            serde_json::json!({"cmd": "LRANGE", "key": key, "start": start, "stop": stop}),
        )?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// LLEN key
    pub fn llen(&self, key: &str) -> Result<usize, String> {
        let result = self.execute(serde_json::json!({"cmd": "LLEN", "key": key}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "LLEN failed".into())
    }

    // -----------------------------------------------------------------------
    // Set operations
    // -----------------------------------------------------------------------

    /// SADD key member
    pub fn sadd(&self, key: &str, member: &str) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "SADD", "key": key, "member": member}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// SREM key member
    pub fn srem(&self, key: &str, member: &str) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "SREM", "key": key, "member": member}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// SMEMBERS key
    pub fn smembers(&self, key: &str) -> Result<Vec<String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "SMEMBERS", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// SISMEMBER key member
    pub fn sismember(&self, key: &str, member: &str) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "SISMEMBER", "key": key, "member": member}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// SCARD key
    pub fn scard(&self, key: &str) -> Result<usize, String> {
        let result = self.execute(serde_json::json!({"cmd": "SCARD", "key": key}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "SCARD failed".into())
    }

    // -----------------------------------------------------------------------
    // Hash operations
    // -----------------------------------------------------------------------

    /// HSET key field value
    pub fn hset(&self, key: &str, field: &str, value: &str) -> Result<(), String> {
        self.execute(
            serde_json::json!({"cmd": "HSET", "key": key, "field": field, "value": value}),
        )?;
        Ok(())
    }

    /// HGET key field
    pub fn hget(&self, key: &str, field: &str) -> Result<Option<String>, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "HGET", "key": key, "field": field}))?;
        Ok(result.get("result").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        }))
    }

    /// HDEL key field
    pub fn hdel(&self, key: &str, field: &str) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "HDEL", "key": key, "field": field}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// HGETALL key
    pub fn hgetall(&self, key: &str) -> Result<HashMap<String, String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "HGETALL", "key": key}))?;
        let mut map = HashMap::new();
        if let Some(obj) = result.get("result").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    map.insert(k.clone(), s.to_string());
                }
            }
        }
        Ok(map)
    }

    /// HEXISTS key field
    pub fn hexists(&self, key: &str, field: &str) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "HEXISTS", "key": key, "field": field}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// HLEN key
    pub fn hlen(&self, key: &str) -> Result<usize, String> {
        let result = self.execute(serde_json::json!({"cmd": "HLEN", "key": key}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "HLEN failed".into())
    }

    /// HKEYS key
    pub fn hkeys(&self, key: &str) -> Result<Vec<String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "HKEYS", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// HINCRBY key field amount
    pub fn hincrby(&self, key: &str, field: &str, amount: i64) -> Result<i64, String> {
        let result = self.execute(
            serde_json::json!({"cmd": "HINCRBY", "key": key, "field": field, "amount": amount}),
        )?;
        result
            .get("result")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("HINCRBY failed")
                    .to_string()
            })
    }

    // -----------------------------------------------------------------------
    // Sorted set operations
    // -----------------------------------------------------------------------

    /// ZADD key score member
    pub fn zadd(&self, key: &str, score: f64, member: &str) -> Result<(), String> {
        self.execute(
            serde_json::json!({"cmd": "ZADD", "key": key, "score": score, "member": member}),
        )?;
        Ok(())
    }

    /// ZREM key member
    pub fn zrem(&self, key: &str, member: &str) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "ZREM", "key": key, "member": member}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// ZSCORE key member
    pub fn zscore(&self, key: &str, member: &str) -> Result<Option<f64>, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "ZSCORE", "key": key, "member": member}))?;
        Ok(result
            .get("result")
            .and_then(|v| if v.is_null() { None } else { v.as_f64() }))
    }

    /// ZRANK key member
    pub fn zrank(&self, key: &str, member: &str) -> Result<Option<usize>, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "ZRANK", "key": key, "member": member}))?;
        Ok(result.get("result").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_u64().map(|n| n as usize)
            }
        }))
    }

    /// ZRANGE key start stop
    pub fn zrange(
        &self,
        key: &str,
        start: usize,
        stop: usize,
    ) -> Result<Vec<(String, f64)>, String> {
        let result = self.execute(
            serde_json::json!({"cmd": "ZRANGE", "key": key, "start": start, "stop": stop}),
        )?;
        let mut entries = Vec::new();
        if let Some(arr) = result.get("result").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    let member = obj
                        .get("member")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let score = obj.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    entries.push((member, score));
                }
            }
        }
        Ok(entries)
    }

    /// ZCARD key
    pub fn zcard(&self, key: &str) -> Result<usize, String> {
        let result = self.execute(serde_json::json!({"cmd": "ZCARD", "key": key}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "ZCARD failed".into())
    }

    // -----------------------------------------------------------------------
    // Utility operations
    // -----------------------------------------------------------------------

    /// KEYS pattern
    pub fn keys(&self, pattern: &str) -> Result<Vec<String>, String> {
        let result = self.execute(serde_json::json!({"cmd": "KEYS", "pattern": pattern}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// TTL key
    pub fn ttl(&self, key: &str) -> Result<Option<u64>, String> {
        let result = self.execute(serde_json::json!({"cmd": "TTL", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| if v.is_null() { None } else { v.as_u64() }))
    }

    /// EXPIRE key seconds
    pub fn expire(&self, key: &str, seconds: u64) -> Result<bool, String> {
        let result =
            self.execute(serde_json::json!({"cmd": "EXPIRE", "key": key, "seconds": seconds}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// PERSIST key
    pub fn persist(&self, key: &str) -> Result<bool, String> {
        let result = self.execute(serde_json::json!({"cmd": "PERSIST", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// TYPE key
    pub fn key_type(&self, key: &str) -> Result<String, String> {
        let result = self.execute(serde_json::json!({"cmd": "TYPE", "key": key}))?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string())
    }

    /// DBSIZE
    pub fn dbsize(&self) -> Result<usize, String> {
        let result = self.execute(serde_json::json!({"cmd": "DBSIZE"}))?;
        result
            .get("result")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| "DBSIZE failed".into())
    }

    /// FLUSHALL
    pub fn flushall(&self) -> Result<(), String> {
        self.execute(serde_json::json!({"cmd": "FLUSHALL"}))?;
        Ok(())
    }

    /// INFO
    pub fn info(&self) -> Result<serde_json::Value, String> {
        let result = self.execute(serde_json::json!({"cmd": "INFO"}))?;
        Ok(result
            .get("result")
            .cloned()
            .unwrap_or(serde_json::json!({})))
    }

    // -----------------------------------------------------------------------
    // Pub/Sub
    // -----------------------------------------------------------------------

    /// Publish a message to a channel on the remote cache server.
    /// Returns the number of subscribers notified.
    pub fn publish(&self, channel: &str, message: &str) -> Result<usize, String> {
        let result = self.post(
            "/pubsub/publish",
            &serde_json::json!({"channel": channel, "message": message}),
        )?;
        Ok(result
            .get("subscribers")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize)
    }

    /// List channels with subscriber counts.
    pub fn channels(&self) -> Result<Vec<(String, usize)>, String> {
        let result = self.get("/pubsub/channels")?;
        let mut channels = Vec::new();
        if let Some(arr) = result.get("result").and_then(|v| v.as_array()) {
            for item in arr {
                let ch = item
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let count = item
                    .get("subscribers")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                channels.push((ch, count));
            }
        }
        Ok(channels)
    }

    /// Get message history for a channel.
    pub fn history(&self, channel: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
        let path = format!("/pubsub/history/{channel}?limit={limit}");
        let result = self.get(&path)?;
        Ok(result
            .get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Health check -- returns the server's health response.
    pub fn health(&self) -> Result<serde_json::Value, String> {
        self.get("/health")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_base_url() {
        let c = RemoteCacheClient::new("http://localhost:6380");
        assert_eq!(c.host_port, "localhost:6380");
    }

    #[test]
    fn parse_base_url_trailing_slash() {
        let c = RemoteCacheClient::new("http://localhost:6380/");
        assert_eq!(c.host_port, "localhost:6380");
    }

    #[test]
    fn parse_base_url_no_scheme() {
        let c = RemoteCacheClient::new("cache.internal:6380");
        assert_eq!(c.host_port, "cache.internal:6380");
    }

    #[test]
    fn parse_base_url_with_path() {
        let c = RemoteCacheClient::new("http://localhost:6380/some/path");
        assert_eq!(c.host_port, "localhost:6380");
    }
}
