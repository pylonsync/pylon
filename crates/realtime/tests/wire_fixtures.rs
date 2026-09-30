//! Wire protocol version 2 frames and input envelopes shared with the
//! TypeScript, Swift, and C# clients (packages/realtime/src/wire.fixtures.json).
//!
//! Each frame is what the server sends: `frame_v2` around a payload that
//! the server's own encoders wrote (serde_json, and rmp-serde with named
//! fields as `snapshot.rs` uses). Each input is an envelope `{ input,
//! client_seq }` as JSON text and as MessagePack; a client must produce
//! those MessagePack bytes exactly, and the server must decode both. The
//! file must match what the encoders produce now: change the format, and
//! this test fails until the fixtures are regenerated with
//! `PYLON_WRITE_FIXTURES=1 cargo test -p pylon-realtime --test wire_fixtures`.

use std::path::PathBuf;

use pylon_realtime::wire::{codec, frame_v2, kind, InputEnvelope, InputRejection, TransferNotice};
use serde_json::{json, Value};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn msgpack<T: serde::Serialize>(value: &T) -> Vec<u8> {
    rmp_serde::to_vec_named(value).unwrap()
}

/// A payload with every value type a snapshot carries: signed and
/// unsigned integers of each width, floats, strings of each length class,
/// nested arrays and maps, booleans, and null.
fn state() -> Value {
    json!({
        "tick_rate": 20,
        "players": [
            { "id": "p1", "x": 12.5, "y": -3.25, "hp": 100, "alive": true },
            { "id": "p2", "x": 0.0, "y": 1e-7, "hp": 0, "alive": false }
        ],
        "ints": [0, 1, 127, 128, 255, 256, 65535, 65536, 4294967295u64, 4294967296u64,
                 -1, -32, -33, -128, -129, -32768, -32769, -2147483648i64, -2147483649i64,
                 9007199254740991u64],
        "floats": [0.5, -1.5, 3.141592653589793, 1e300],
        "strings": ["", "a", "é ü 中文", "a string that is longer than thirty-one bytes, so it needs str8"],
        "nothing": null,
        "nested": { "a": { "b": { "c": [1, [2, [3]]] } } }
    })
}

fn frame(kind_: u8, codec_: u8, tick: u64, ack: u64, payload: &[u8], expect: Value) -> Value {
    json!({
        "frame": hex(&frame_v2(kind_, codec_, tick, ack, payload)),
        "kind": kind_,
        "codec": codec_,
        "tick": tick,
        "ack": ack,
        "payload": expect,
    })
}

fn frames() -> Value {
    let s = state();
    let rejected = InputRejection {
        client_seq: Some(42),
        code: "rate_limited".into(),
        message: "too many inputs".into(),
    };
    let rejected_no_seq = InputRejection {
        client_seq: None,
        code: "invalid".into(),
        message: "input did not decode".into(),
    };
    let transfer = TransferNotice {
        shard: "zone-2".into(),
        ticket: "v1.eyJleHAiOjF9.sig".into(),
    };
    Value::Array(vec![
        frame(
            kind::SNAPSHOT,
            codec::JSON,
            1,
            0,
            s.to_string().as_bytes(),
            s.clone(),
        ),
        frame(
            kind::SNAPSHOT,
            codec::MESSAGE_PACK,
            7,
            3,
            &msgpack(&s),
            s.clone(),
        ),
        // The largest tick and ack every client holds exactly (2^53 - 1).
        frame(
            kind::SNAPSHOT,
            codec::JSON,
            9007199254740991,
            9007199254740990,
            b"{}",
            json!({}),
        ),
        frame(
            kind::INPUT_REJECTED,
            codec::JSON,
            8,
            41,
            serde_json::to_string(&rejected).unwrap().as_bytes(),
            serde_json::to_value(&rejected).unwrap(),
        ),
        frame(
            kind::INPUT_REJECTED,
            codec::MESSAGE_PACK,
            8,
            41,
            &msgpack(&rejected_no_seq),
            serde_json::to_value(&rejected_no_seq).unwrap(),
        ),
        frame(
            kind::TRANSFER,
            codec::JSON,
            9,
            41,
            serde_json::to_string(&transfer).unwrap().as_bytes(),
            serde_json::to_value(&transfer).unwrap(),
        ),
    ])
}

#[derive(serde::Serialize)]
struct Envelope<'a> {
    input: &'a Value,
    client_seq: u64,
}

fn inputs() -> Value {
    let cases = [
        (json!("join"), 1u64),
        (json!({ "move_to": { "x": 120.5, "y": 64.0 } }), 2),
        (
            json!({ "drive": { "pos": [20, 0, -20], "speed": 20.25, "air": false } }),
            300,
        ),
        (json!([1, -1, 70000, "x", null, true]), 70000),
        (json!({}), 4294967296),
    ];
    Value::Array(
        cases
            .iter()
            .map(|(input, seq)| {
                let env = Envelope {
                    input,
                    client_seq: *seq,
                };
                let text = serde_json::to_string(&env).unwrap();
                let packed = msgpack(&env);
                // The server decodes both forms to the same envelope.
                let from_json: InputEnvelope<Value> = serde_json::from_str(&text).unwrap();
                let from_msgpack: InputEnvelope<Value> = rmp_serde::from_slice(&packed).unwrap();
                assert_eq!(&from_json.input, input);
                assert_eq!(&from_msgpack.input, input);
                assert_eq!(from_json.client_seq, Some(*seq));
                assert_eq!(from_msgpack.client_seq, Some(*seq));
                json!({ "input": input, "client_seq": seq, "json": text, "msgpack": hex(&packed) })
            })
            .collect(),
    )
}

fn fixtures() -> Value {
    json!({ "version": 2, "headerLength": 18, "frames": frames(), "inputs": inputs() })
}

#[test]
fn the_fixture_file_matches_the_encoders() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/realtime/src/wire.fixtures.json");
    let now = fixtures();
    if std::env::var("PYLON_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .expect("no fixture file; run with PYLON_WRITE_FIXTURES=1 to write it"),
    )
    .unwrap();
    assert_eq!(
        on_disk, now,
        "the wire fixtures are out of date; run with PYLON_WRITE_FIXTURES=1"
    );
}
