//! The shard wire protocol.
//!
//! A client picks the version with the `v` query parameter when it opens
//! the shard WebSocket (`?shard=<id>&sid=<subscriber>&v=2`). Without `v`,
//! the server speaks version 1.
//!
//! # Version 2
//!
//! Every server-to-client message is a binary frame with an 18-byte header:
//!
//! | Offset | Size | Field |
//! | --- | --- | --- |
//! | 0 | 1 | Frame kind: `1` snapshot, `2` input rejected, `3` replication |
//! | 1 | 1 | Codec of the payload: `0` JSON, `1` MessagePack, `2` bincode, `3` custom, `4` replication |
//! | 2 | 8 | Tick number, u64 big-endian |
//! | 10 | 8 | Ack: the highest `client_seq` the shard has processed for this subscriber, u64 big-endian, `0` when none |
//! | 18 | .. | Payload |
//!
//! - A snapshot payload is the subscriber's snapshot in the shard's codec.
//! - An input-rejected payload is an [`InputRejection`] in the shard's codec:
//!   `{ client_seq, code, message }`. The shard sends one when an input is
//!   refused (authorization, rate limit, a full queue, bad encoding) or when
//!   `SimState::apply_input` returns an error.
//! - The ack covers every processed input, applied or rejected. A client
//!   that predicts locally drops predictions up to the ack and replays the
//!   rest on top of the snapshot.
//! - MessagePack payloads encode structs as maps with field names, so any
//!   MessagePack library decodes them into keyed objects. Bincode has no
//!   schema on the wire and suits Rust clients only. Codec `3` is reserved
//!   for a game's own encoding.
//!
//! Client-to-server messages carry an input envelope
//! `{ "input": <input>, "client_seq": <n> }` (`client_seq` optional):
//!
//! - a text frame holds it as JSON,
//! - a binary frame holds it in the shard's codec.
//!
//! # Version 1
//!
//! A snapshot frame is the 8-byte big-endian tick followed by the payload,
//! with no codec marker, ack, or rejection frames. Inputs are JSON text or
//! binary frames holding UTF-8 JSON.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::snapshot::SnapshotFormat;

/// Length of the version 2 frame header.
pub const HEADER_LEN: usize = 18;

/// Frame kinds in the version 2 header.
pub mod kind {
    pub const SNAPSHOT: u8 = 1;
    pub const INPUT_REJECTED: u8 = 2;
    /// An entity replication frame (`pylon_replication::frame`). Its codec
    /// byte is [`super::codec::REPLICATION`].
    pub const REPLICATION: u8 = 3;
}

/// Codec bytes in the version 2 header.
pub mod codec {
    pub const JSON: u8 = 0;
    pub const MESSAGE_PACK: u8 = 1;
    pub const BINCODE: u8 = 2;
    /// Reserved for a game's own encoding.
    pub const CUSTOM: u8 = 3;
    /// The replication frame format, version 1.
    pub const REPLICATION: u8 = 4;
}

/// The codec byte for a snapshot format.
pub fn codec_byte(format: SnapshotFormat) -> u8 {
    match format {
        SnapshotFormat::Json | SnapshotFormat::JsonCompact => codec::JSON,
        SnapshotFormat::MessagePack => codec::MESSAGE_PACK,
        SnapshotFormat::Bincode => codec::BINCODE,
    }
}

/// A version 2 frame: header followed by `payload`.
pub fn frame_v2(kind: u8, codec: u8, tick: u64, ack: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(kind);
    out.push(codec);
    out.extend_from_slice(&tick.to_be_bytes());
    out.extend_from_slice(&ack.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// A version 1 snapshot frame: tick followed by `payload`.
pub fn frame_v1(tick: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&tick.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Why an input did not take effect. Sent as an input-rejected frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRejection {
    /// The client's sequence number for the input, when it sent one.
    pub client_seq: Option<u64>,
    /// One of `unauthorized`, `rate_limited`, `queue_full`, `invalid`,
    /// `stopped`, `apply_failed`.
    pub code: String,
    pub message: String,
}

/// A client input with its optional sequence number.
#[derive(Debug, Clone, Deserialize)]
pub struct InputEnvelope<I> {
    pub input: I,
    #[serde(default)]
    pub client_seq: Option<u64>,
}

/// Decode an input envelope in `format`.
pub fn decode_input_envelope<I: DeserializeOwned>(
    format: SnapshotFormat,
    bytes: &[u8],
) -> Result<InputEnvelope<I>, String> {
    match format {
        SnapshotFormat::Json | SnapshotFormat::JsonCompact => {
            serde_json::from_slice(bytes).map_err(|e| format!("json: {e}"))
        }
        SnapshotFormat::MessagePack => {
            #[cfg(feature = "msgpack")]
            {
                rmp_serde::from_slice(bytes).map_err(|e| format!("msgpack: {e}"))
            }
            #[cfg(not(feature = "msgpack"))]
            {
                Err("MessagePack requires the `msgpack` feature on pylon-realtime".into())
            }
        }
        SnapshotFormat::Bincode => {
            #[cfg(feature = "bincode")]
            {
                // Bincode has no field names: the envelope is (input, client_seq).
                let (input, client_seq): (I, Option<u64>) =
                    bincode::deserialize(bytes).map_err(|e| format!("bincode: {e}"))?;
                Ok(InputEnvelope { input, client_seq })
            }
            #[cfg(not(feature = "bincode"))]
            {
                Err("bincode requires the `bincode` feature on pylon-realtime".into())
            }
        }
    }
}

/// A shard input type: how the shard turns wire bytes into `Self`.
///
/// Every `DeserializeOwned` type is a `ShardInput` through
/// [`decode_input_envelope`]. [`crate::raw::RawInput`] implements it by hand
/// to keep the input bytes undecoded for a WebAssembly shard.
pub trait ShardInput: Sized + Send + 'static {
    /// Decode an envelope `{ input, client_seq? }` that arrived in `wire`
    /// (JSON for a text frame, the shard's codec for a binary frame).
    /// `shard` is the shard's codec.
    fn decode_envelope(
        wire: SnapshotFormat,
        shard: SnapshotFormat,
        bytes: &[u8],
    ) -> Result<InputEnvelope<Self>, String>;

    /// Decode a bare JSON input (the HTTP input route).
    fn decode_json(body: &str, shard: SnapshotFormat) -> Result<Self, String>;
}

impl<T: DeserializeOwned + Send + 'static> ShardInput for T {
    fn decode_envelope(
        wire: SnapshotFormat,
        _shard: SnapshotFormat,
        bytes: &[u8],
    ) -> Result<InputEnvelope<Self>, String> {
        decode_input_envelope(wire, bytes)
    }

    fn decode_json(body: &str, _shard: SnapshotFormat) -> Result<Self, String> {
        serde_json::from_str(body).map_err(|e| format!("invalid input JSON: {e}"))
    }
}

/// Only the `client_seq` of an envelope, for reporting a rejection when the
/// input itself does not decode.
pub fn peek_client_seq(format: SnapshotFormat, bytes: &[u8]) -> Option<u64> {
    #[derive(Deserialize)]
    struct Seq {
        #[serde(default)]
        client_seq: Option<u64>,
    }
    match format {
        SnapshotFormat::Json | SnapshotFormat::JsonCompact => {
            serde_json::from_slice::<Seq>(bytes).ok()?.client_seq
        }
        #[cfg(feature = "msgpack")]
        SnapshotFormat::MessagePack => rmp_serde::from_slice::<Seq>(bytes).ok()?.client_seq,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_header_layout() {
        let f = frame_v2(kind::SNAPSHOT, codec::MESSAGE_PACK, 0x0102, 7, b"xy");
        assert_eq!(f.len(), HEADER_LEN + 2);
        assert_eq!(f[0], 1);
        assert_eq!(f[1], 1);
        assert_eq!(u64::from_be_bytes(f[2..10].try_into().unwrap()), 0x0102);
        assert_eq!(u64::from_be_bytes(f[10..18].try_into().unwrap()), 7);
        assert_eq!(&f[18..], b"xy");
    }

    #[test]
    fn json_envelope_with_and_without_seq() {
        let e: InputEnvelope<i64> =
            decode_input_envelope(SnapshotFormat::Json, br#"{"input":5,"client_seq":9}"#).unwrap();
        assert_eq!((e.input, e.client_seq), (5, Some(9)));
        let e: InputEnvelope<i64> =
            decode_input_envelope(SnapshotFormat::Json, br#"{"input":5}"#).unwrap();
        assert_eq!(e.client_seq, None);
        assert_eq!(
            peek_client_seq(SnapshotFormat::Json, br#"{"input":"bad","client_seq":3}"#),
            Some(3)
        );
    }

    #[cfg(feature = "msgpack")]
    #[test]
    fn msgpack_envelope_round_trip() {
        #[derive(Serialize)]
        struct Env {
            input: (i32, String),
            client_seq: u64,
        }
        let bytes = rmp_serde::to_vec_named(&Env {
            input: (3, "go".into()),
            client_seq: 4,
        })
        .unwrap();
        let e: InputEnvelope<(i32, String)> =
            decode_input_envelope(SnapshotFormat::MessagePack, &bytes).unwrap();
        assert_eq!(e.input, (3, "go".to_string()));
        assert_eq!(e.client_seq, Some(4));
    }
}
