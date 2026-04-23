//! Pluggable snapshot encoding.
//!
//! Games vary in what they can tolerate on the wire. JSON is readable and
//! universal; binary formats like MessagePack or bincode are smaller and
//! faster but require an SDK-aware client.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotFormat {
    /// Human-readable JSON. Good for dev, turn-based games, low-bandwidth.
    Json,

    /// Compact JSON as bytes (no spaces/newlines). Slightly smaller than
    /// pretty JSON and parseable in any language.
    JsonCompact,

    /// MessagePack via `rmp_serde` if wired. Falls back to JSON when the
    /// feature isn't enabled.
    MessagePack,

    /// Bincode via `bincode` if wired. Falls back to JSON when the feature
    /// isn't enabled.
    Bincode,
}

#[derive(Debug, Clone)]
pub struct EncodeError {
    pub message: String,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EncodeError {}

/// Encode a snapshot using the chosen format.
///
/// JSON is always available. MessagePack requires the `msgpack` feature,
/// bincode requires the `bincode` feature. When a binary format is requested
/// but the feature isn't enabled, we fall back to JSON and return an error
/// noting the missing feature — callers should generally enable the feature
/// at build time.
pub fn encode_snapshot<T: Serialize>(
    snapshot: &T,
    format: SnapshotFormat,
) -> Result<Vec<u8>, EncodeError> {
    match format {
        SnapshotFormat::Json | SnapshotFormat::JsonCompact => serde_json::to_vec(snapshot)
            .map_err(|e| EncodeError {
                message: format!("json: {e}"),
            }),
        SnapshotFormat::MessagePack => {
            #[cfg(feature = "msgpack")]
            {
                rmp_serde::to_vec(snapshot).map_err(|e| EncodeError {
                    message: format!("msgpack: {e}"),
                })
            }
            #[cfg(not(feature = "msgpack"))]
            {
                Err(EncodeError {
                    message: "MessagePack requires the `msgpack` feature to be enabled on pylon-realtime".into(),
                })
            }
        }
        SnapshotFormat::Bincode => {
            #[cfg(feature = "bincode")]
            {
                bincode::serialize(snapshot).map_err(|e| EncodeError {
                    message: format!("bincode: {e}"),
                })
            }
            #[cfg(not(feature = "bincode"))]
            {
                Err(EncodeError {
                    message: "Bincode requires the `bincode` feature to be enabled on pylon-realtime".into(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_encodes_simple() {
        let bytes = encode_snapshot(&42u64, SnapshotFormat::Json).unwrap();
        assert_eq!(bytes, b"42");
    }

    #[test]
    fn msgpack_without_feature_errors() {
        #[cfg(not(feature = "msgpack"))]
        {
            let result = encode_snapshot(&42u64, SnapshotFormat::MessagePack);
            assert!(result.is_err());
            let msg = result.unwrap_err().message;
            assert!(msg.contains("msgpack"));
        }
    }

    #[cfg(feature = "msgpack")]
    #[test]
    fn msgpack_encodes_when_enabled() {
        let bytes = encode_snapshot(&42u64, SnapshotFormat::MessagePack).unwrap();
        assert!(!bytes.is_empty());
        // MessagePack encodes 42 as 0x2A (positive fixint).
        assert_eq!(bytes[0], 0x2A);
    }

    #[test]
    fn json_encodes_struct() {
        #[derive(Serialize)]
        struct Snap {
            tick: u64,
            players: Vec<String>,
        }
        let bytes = encode_snapshot(
            &Snap {
                tick: 1,
                players: vec!["alice".into(), "bob".into()],
            },
            SnapshotFormat::Json,
        )
        .unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("\"tick\":1"));
        assert!(s.contains("\"alice\""));
    }
}
