//! Inputs and snapshots kept as bytes in the shard's codec.
//!
//! A [`crate::SimState`] written in Rust uses its own serde types. A
//! simulation that runs somewhere else (a WebAssembly module) encodes and
//! decodes for itself, so the shard must hand it input bytes and send its
//! snapshot bytes unchanged. [`RawInput`] and [`RawSnapshot`] carry those
//! bytes.
//!
//! Only JSON and MessagePack shards can use them: the input envelope must be
//! self-describing so the shard can cut the `input` value out of it.

use std::ops::Range;
use std::sync::Arc;

use serde::Deserialize;

use crate::snapshot::{EncodeError, EncodeSnapshot, SnapshotFormat};
use crate::wire::{InputEnvelope, ShardInput};

/// One input, encoded in the shard's codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawInput {
    format: SnapshotFormat,
    bytes: Arc<[u8]>,
}

impl RawInput {
    pub fn new(format: SnapshotFormat, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            format,
            bytes: bytes.into(),
        }
    }

    pub fn format(&self) -> SnapshotFormat {
        self.format
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl ShardInput for RawInput {
    fn decode_envelope(
        wire: SnapshotFormat,
        shard: SnapshotFormat,
        bytes: &[u8],
    ) -> Result<InputEnvelope<Self>, String> {
        let (range, client_seq) = split_envelope(wire, bytes)?;
        let input = convert(wire, shard, &bytes[range])?;
        Ok(InputEnvelope { input, client_seq })
    }

    fn decode_json(body: &str, shard: SnapshotFormat) -> Result<Self, String> {
        // Reject malformed JSON here, not in the simulation.
        let _: serde::de::IgnoredAny =
            serde_json::from_str(body).map_err(|e| format!("invalid input JSON: {e}"))?;
        convert(SnapshotFormat::Json, shard, body.as_bytes())
    }
}

/// A snapshot, already encoded in the shard's codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSnapshot {
    format: SnapshotFormat,
    bytes: Arc<[u8]>,
}

impl RawSnapshot {
    pub fn new(format: SnapshotFormat, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            format,
            bytes: bytes.into(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl EncodeSnapshot for RawSnapshot {
    fn encode_as(&self, format: SnapshotFormat) -> Result<Vec<u8>, EncodeError> {
        if same_codec(self.format, format) {
            Ok(self.bytes.to_vec())
        } else {
            Err(EncodeError {
                message: format!(
                    "raw snapshot is {:?}, but the shard sends {:?}",
                    self.format, format
                ),
            })
        }
    }
}

fn same_codec(a: SnapshotFormat, b: SnapshotFormat) -> bool {
    use SnapshotFormat::*;
    matches!(
        (a, b),
        (Json | JsonCompact, Json | JsonCompact) | (MessagePack, MessagePack) | (Bincode, Bincode)
    )
}

/// Re-encode an input value from the wire codec into the shard codec. A JSON
/// text frame to a MessagePack shard is the one case that needs work.
fn convert(wire: SnapshotFormat, shard: SnapshotFormat, bytes: &[u8]) -> Result<RawInput, String> {
    if same_codec(wire, shard) {
        return Ok(RawInput::new(shard, bytes));
    }
    match (wire, shard) {
        (SnapshotFormat::Json | SnapshotFormat::JsonCompact, SnapshotFormat::MessagePack) => {
            #[cfg(feature = "msgpack")]
            {
                let value: serde_json::Value =
                    serde_json::from_slice(bytes).map_err(|e| format!("json: {e}"))?;
                let packed =
                    rmp_serde::to_vec_named(&value).map_err(|e| format!("msgpack: {e}"))?;
                Ok(RawInput::new(shard, packed))
            }
            #[cfg(not(feature = "msgpack"))]
            {
                Err("MessagePack requires the `msgpack` feature on pylon-realtime".into())
            }
        }
        _ => Err(format!(
            "a {wire:?} input cannot go to a {shard:?} shard; send JSON or the shard's codec"
        )),
    }
}

/// Find the `input` value inside an envelope without decoding it. Returns
/// its byte range and the envelope's `client_seq`.
fn split_envelope(
    wire: SnapshotFormat,
    bytes: &[u8],
) -> Result<(Range<usize>, Option<u64>), String> {
    match wire {
        SnapshotFormat::Json | SnapshotFormat::JsonCompact => split_json(bytes),
        SnapshotFormat::MessagePack => {
            #[cfg(feature = "msgpack")]
            {
                split_msgpack(bytes)
            }
            #[cfg(not(feature = "msgpack"))]
            {
                Err("MessagePack requires the `msgpack` feature on pylon-realtime".into())
            }
        }
        SnapshotFormat::Bincode => Err(
            "bincode inputs have no field names; a raw-input shard needs JSON or MessagePack"
                .into(),
        ),
    }
}

fn split_json(bytes: &[u8]) -> Result<(Range<usize>, Option<u64>), String> {
    #[derive(Deserialize)]
    struct Envelope<'a> {
        #[serde(borrow)]
        input: &'a serde_json::value::RawValue,
        #[serde(default)]
        client_seq: Option<u64>,
    }
    let env: Envelope = serde_json::from_slice(bytes).map_err(|e| format!("json: {e}"))?;
    let raw = env.input.get().as_bytes();
    // `raw` borrows from `bytes`, so its offset is the pointer difference.
    let start = raw.as_ptr() as usize - bytes.as_ptr() as usize;
    Ok((start..start + raw.len(), env.client_seq))
}

#[cfg(feature = "msgpack")]
fn split_msgpack(bytes: &[u8]) -> Result<(Range<usize>, Option<u64>), String> {
    use serde::de::IgnoredAny;

    let err = |e: &dyn std::fmt::Display| format!("msgpack: {e}");
    let mut rd: &[u8] = bytes;
    let len = rmp::decode::read_map_len(&mut rd).map_err(|e| err(&e))?;
    let mut input = None;
    let mut client_seq = None;
    for _ in 0..len {
        let key_len = rmp::decode::read_str_len(&mut rd).map_err(|e| err(&e))? as usize;
        if rd.len() < key_len {
            return Err("msgpack: envelope key runs past the end".into());
        }
        let (key, rest) = rd.split_at(key_len);
        rd = rest;
        let start = bytes.len() - rd.len();
        match key {
            b"input" => {
                IgnoredAny::deserialize(&mut rmp_serde::Deserializer::new(&mut rd))
                    .map_err(|e| err(&e))?;
                input = Some(start..bytes.len() - rd.len());
            }
            b"client_seq" => {
                client_seq = Option::<u64>::deserialize(&mut rmp_serde::Deserializer::new(&mut rd))
                    .map_err(|e| err(&e))?;
            }
            _ => {
                IgnoredAny::deserialize(&mut rmp_serde::Deserializer::new(&mut rd))
                    .map_err(|e| err(&e))?;
            }
        }
    }
    let input = input.ok_or_else(|| "msgpack: missing field `input`".to_string())?;
    Ok((input, client_seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_envelope_keeps_the_input_bytes() {
        let body = br#"{"client_seq":4,"input":{"dx": 1, "dy":[2,3]}}"#;
        let env =
            RawInput::decode_envelope(SnapshotFormat::Json, SnapshotFormat::Json, body).unwrap();
        assert_eq!(env.client_seq, Some(4));
        assert_eq!(env.input.bytes(), br#"{"dx": 1, "dy":[2,3]}"#);
        assert_eq!(env.input.format(), SnapshotFormat::Json);
    }

    #[test]
    fn json_envelope_without_input_is_invalid() {
        let err = RawInput::decode_envelope(
            SnapshotFormat::Json,
            SnapshotFormat::Json,
            br#"{"client_seq":1}"#,
        )
        .unwrap_err();
        assert!(err.contains("input"), "{err}");
    }

    #[test]
    fn bare_json_input_is_checked() {
        let input = RawInput::decode_json(r#"{"a":1}"#, SnapshotFormat::Json).unwrap();
        assert_eq!(input.bytes(), br#"{"a":1}"#);
        assert!(RawInput::decode_json("{nope", SnapshotFormat::Json)
            .unwrap_err()
            .contains("invalid input JSON"));
    }

    #[test]
    fn snapshot_passes_through_in_its_own_codec_only() {
        let snap = RawSnapshot::new(SnapshotFormat::Json, &b"{\"t\":1}"[..]);
        assert_eq!(
            snap.encode_as(SnapshotFormat::JsonCompact).unwrap(),
            b"{\"t\":1}"
        );
        assert!(snap.encode_as(SnapshotFormat::MessagePack).is_err());
    }

    #[cfg(feature = "msgpack")]
    #[test]
    fn msgpack_envelope_keeps_the_input_bytes() {
        #[derive(serde::Serialize)]
        struct Env {
            extra: Vec<u8>,
            input: (i32, String),
            client_seq: u64,
        }
        let body = rmp_serde::to_vec_named(&Env {
            extra: vec![1, 2],
            input: (7, "go".into()),
            client_seq: 11,
        })
        .unwrap();
        let env = RawInput::decode_envelope(
            SnapshotFormat::MessagePack,
            SnapshotFormat::MessagePack,
            &body,
        )
        .unwrap();
        assert_eq!(env.client_seq, Some(11));
        let back: (i32, String) = rmp_serde::from_slice(env.input.bytes()).unwrap();
        assert_eq!(back, (7, "go".into()));
    }

    #[cfg(feature = "msgpack")]
    #[test]
    fn json_frame_to_msgpack_shard_is_re_encoded() {
        let env = RawInput::decode_envelope(
            SnapshotFormat::Json,
            SnapshotFormat::MessagePack,
            br#"{"input":{"dx":1},"client_seq":2}"#,
        )
        .unwrap();
        assert_eq!(env.input.format(), SnapshotFormat::MessagePack);
        let back: serde_json::Value = rmp_serde::from_slice(env.input.bytes()).unwrap();
        assert_eq!(back, serde_json::json!({ "dx": 1 }));
    }

    #[cfg(feature = "msgpack")]
    #[test]
    fn truncated_msgpack_envelope_is_invalid() {
        let body = rmp_serde::to_vec_named(&serde_json::json!({ "input": "abcdef" })).unwrap();
        assert!(RawInput::decode_envelope(
            SnapshotFormat::MessagePack,
            SnapshotFormat::MessagePack,
            &body[..body.len() - 2],
        )
        .is_err());
    }
}
