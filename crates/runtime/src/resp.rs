//! RESP (Redis Serialization Protocol) parser and serializer.
//!
//! Implements RESP2, the wire protocol used by Redis. This allows pylon's
//! cache to be accessed by any standard Redis client library.
//!
//! # Wire format
//!
//! - Simple strings: `+OK\r\n`
//! - Errors:         `-ERR message\r\n`
//! - Integers:       `:1000\r\n`
//! - Bulk strings:   `$5\r\nhello\r\n`  (length-prefixed)
//! - Arrays:         `*2\r\n$3\r\nfoo\r\n$3\r\nbar\r\n`
//! - Null:           `$-1\r\n`

use std::io::BufRead;

/// A RESP value.
#[derive(Debug, Clone, PartialEq)]
pub enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Option<String>),    // None = null bulk string
    Array(Option<Vec<RespValue>>), // None = null array
}

impl RespValue {
    /// Serialize to RESP wire format.
    pub fn serialize(&self) -> Vec<u8> {
        match self {
            RespValue::SimpleString(s) => format!("+{s}\r\n").into_bytes(),
            RespValue::Error(s) => format!("-{s}\r\n").into_bytes(),
            RespValue::Integer(n) => format!(":{n}\r\n").into_bytes(),
            RespValue::BulkString(None) => b"$-1\r\n".to_vec(),
            RespValue::BulkString(Some(s)) => {
                let mut buf = format!("${}\r\n", s.len()).into_bytes();
                buf.extend_from_slice(s.as_bytes());
                buf.extend_from_slice(b"\r\n");
                buf
            }
            RespValue::Array(None) => b"*-1\r\n".to_vec(),
            RespValue::Array(Some(items)) => {
                let mut buf = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    buf.extend_from_slice(&item.serialize());
                }
                buf
            }
        }
    }

    /// Create a bulk string value.
    pub fn bulk(s: &str) -> Self {
        RespValue::BulkString(Some(s.to_string()))
    }

    /// Create a null bulk string.
    pub fn null() -> Self {
        RespValue::BulkString(None)
    }

    /// Create a simple string "OK".
    pub fn ok() -> Self {
        RespValue::SimpleString("OK".to_string())
    }

    /// Create an integer value.
    pub fn int(n: i64) -> Self {
        RespValue::Integer(n)
    }

    /// Create an error value with the standard "ERR" prefix.
    pub fn err(msg: &str) -> Self {
        RespValue::Error(format!("ERR {msg}"))
    }

    /// Create an array value.
    pub fn array(items: Vec<RespValue>) -> Self {
        RespValue::Array(Some(items))
    }
}

/// Hard caps on attacker-declared sizes, enforced BEFORE allocation. The
/// RESP parser runs before the AUTH gate (`resp_server` parses the command
/// frame, then checks the password), so an unauthenticated client must not
/// be able to make us allocate — or recurse — without bound from a single
/// frame. Values mirror Redis defaults where applicable
/// (`proto-max-bulk-len` = 512 MiB, multi-bulk = 1M elements).
const MAX_BULK_LEN: usize = 512 * 1024 * 1024;
const MAX_ARRAY_COUNT: usize = 1024 * 1024;
/// Cap on a single header/inline line (`$<len>\r\n`, `+<status>\r\n`, …).
/// Without it, a stream that never sends `\n` grows `line` without bound.
const MAX_LINE_LEN: u64 = 64 * 1024;
/// RESP command frames are flat (`*N` of bulk strings); deep nesting only
/// appears in a malicious frame and would otherwise recurse to stack
/// exhaustion.
const MAX_DEPTH: usize = 32;

/// Parse a single RESP value from a buffered reader.
///
/// Returns `Err` on I/O errors, malformed input, or EOF.
pub fn parse_resp<R: BufRead>(reader: &mut R) -> Result<RespValue, String> {
    parse_resp_depth(reader, 0)
}

fn parse_resp_depth<R: BufRead>(reader: &mut R, depth: usize) -> Result<RespValue, String> {
    if depth > MAX_DEPTH {
        return Err(format!("RESP nesting too deep (> {MAX_DEPTH})"));
    }
    let mut line = String::new();
    // Bound the header/inline line read — past `MAX_LINE_LEN` the line
    // can't end with `\r\n`, so the malformed check below rejects it
    // instead of letting `line` grow to exhaust memory. UFCS + the
    // `&mut *reader` reborrow so `take` (which consumes its receiver)
    // doesn't move `reader`, which we still need for `read_exact`.
    let bytes_read = std::io::Read::take(&mut *reader, MAX_LINE_LEN)
        .read_line(&mut line)
        .map_err(|e| format!("Read error: {e}"))?;

    if bytes_read == 0 {
        return Err("Connection closed".into());
    }

    // Must have at least type byte + \r\n.
    if line.len() < 3 || !line.ends_with("\r\n") {
        return Err(format!("Malformed RESP line: {:?}", line));
    }

    let content = &line[1..line.len() - 2]; // strip type byte and \r\n

    match line.as_bytes()[0] {
        b'+' => Ok(RespValue::SimpleString(content.to_string())),
        b'-' => Ok(RespValue::Error(content.to_string())),
        b':' => {
            let n: i64 = content
                .parse()
                .map_err(|_| format!("Invalid integer: {content:?}"))?;
            Ok(RespValue::Integer(n))
        }
        b'$' => {
            let len: i64 = content
                .parse()
                .map_err(|_| format!("Invalid bulk length: {content:?}"))?;
            if len < 0 {
                return Ok(RespValue::BulkString(None));
            }
            let len = len as usize;
            if len > MAX_BULK_LEN {
                return Err(format!(
                    "bulk string too large: {len} bytes (max {MAX_BULK_LEN})"
                ));
            }
            let mut buf = vec![0u8; len + 2]; // data + trailing \r\n
            reader
                .read_exact(&mut buf)
                .map_err(|e| format!("Read error: {e}"))?;
            if buf[len] != b'\r' || buf[len + 1] != b'\n' {
                return Err("Missing \\r\\n after bulk string data".into());
            }
            let s = String::from_utf8(buf[..len].to_vec())
                .map_err(|_| "Invalid UTF-8 in bulk string")?;
            Ok(RespValue::BulkString(Some(s)))
        }
        b'*' => {
            let count: i64 = content
                .parse()
                .map_err(|_| format!("Invalid array length: {content:?}"))?;
            if count < 0 {
                return Ok(RespValue::Array(None));
            }
            let count = count as usize;
            if count > MAX_ARRAY_COUNT {
                return Err(format!(
                    "array too large: {count} elements (max {MAX_ARRAY_COUNT})"
                ));
            }
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(parse_resp_depth(reader, depth + 1)?);
            }
            Ok(RespValue::Array(Some(items)))
        }
        other => Err(format!("Unknown RESP type byte: {:?}", other as char)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    /// Helper: parse a RESP value from raw bytes.
    fn parse(input: &[u8]) -> Result<RespValue, String> {
        let mut reader = BufReader::new(input);
        parse_resp(&mut reader)
    }

    /// Helper: serialize then re-parse, asserting roundtrip equality.
    fn roundtrip(value: &RespValue) {
        let bytes = value.serialize();
        let parsed = parse(&bytes).expect("roundtrip parse failed");
        assert_eq!(&parsed, value, "roundtrip mismatch");
    }

    // -- Simple strings --

    #[test]
    fn parse_simple_string() {
        let val = parse(b"+OK\r\n").unwrap();
        assert_eq!(val, RespValue::SimpleString("OK".into()));
    }

    #[test]
    fn serialize_simple_string() {
        let val = RespValue::SimpleString("hello world".into());
        assert_eq!(val.serialize(), b"+hello world\r\n");
    }

    #[test]
    fn roundtrip_simple_string() {
        roundtrip(&RespValue::SimpleString("PONG".into()));
        roundtrip(&RespValue::ok());
    }

    // -- Errors --

    #[test]
    fn parse_error() {
        let val = parse(b"-ERR unknown command\r\n").unwrap();
        assert_eq!(val, RespValue::Error("ERR unknown command".into()));
    }

    #[test]
    fn serialize_error() {
        let val = RespValue::err("bad key");
        assert_eq!(val.serialize(), b"-ERR bad key\r\n");
    }

    #[test]
    fn roundtrip_error() {
        roundtrip(&RespValue::err("something went wrong"));
    }

    // -- Integers --

    #[test]
    fn parse_integer() {
        assert_eq!(parse(b":1000\r\n").unwrap(), RespValue::Integer(1000));
        assert_eq!(parse(b":-42\r\n").unwrap(), RespValue::Integer(-42));
        assert_eq!(parse(b":0\r\n").unwrap(), RespValue::Integer(0));
    }

    #[test]
    fn serialize_integer() {
        assert_eq!(RespValue::int(99).serialize(), b":99\r\n");
        assert_eq!(RespValue::int(-1).serialize(), b":-1\r\n");
    }

    #[test]
    fn roundtrip_integer() {
        roundtrip(&RespValue::int(0));
        roundtrip(&RespValue::int(i64::MAX));
        roundtrip(&RespValue::int(i64::MIN));
    }

    // -- Bulk strings --

    #[test]
    fn parse_bulk_string() {
        let val = parse(b"$5\r\nhello\r\n").unwrap();
        assert_eq!(val, RespValue::BulkString(Some("hello".into())));
    }

    #[test]
    fn parse_null_bulk_string() {
        let val = parse(b"$-1\r\n").unwrap();
        assert_eq!(val, RespValue::BulkString(None));
    }

    #[test]
    fn parse_empty_bulk_string() {
        let val = parse(b"$0\r\n\r\n").unwrap();
        assert_eq!(val, RespValue::BulkString(Some(String::new())));
    }

    #[test]
    fn serialize_bulk_string() {
        assert_eq!(RespValue::bulk("foo").serialize(), b"$3\r\nfoo\r\n");
    }

    #[test]
    fn serialize_null_bulk_string() {
        assert_eq!(RespValue::null().serialize(), b"$-1\r\n");
    }

    #[test]
    fn serialize_empty_bulk_string() {
        assert_eq!(
            RespValue::BulkString(Some(String::new())).serialize(),
            b"$0\r\n\r\n"
        );
    }

    #[test]
    fn roundtrip_bulk_string() {
        roundtrip(&RespValue::bulk("hello"));
        roundtrip(&RespValue::null());
        roundtrip(&RespValue::BulkString(Some(String::new())));
    }

    #[test]
    fn large_bulk_string() {
        let large = "x".repeat(100_000);
        let val = RespValue::bulk(&large);
        roundtrip(&val);
    }

    // -- Arrays --

    #[test]
    fn parse_array() {
        let input = b"*2\r\n$3\r\nfoo\r\n$3\r\nbar\r\n";
        let val = parse(input).unwrap();
        assert_eq!(
            val,
            RespValue::Array(Some(vec![RespValue::bulk("foo"), RespValue::bulk("bar"),]))
        );
    }

    #[test]
    fn parse_null_array() {
        let val = parse(b"*-1\r\n").unwrap();
        assert_eq!(val, RespValue::Array(None));
    }

    #[test]
    fn parse_empty_array() {
        let val = parse(b"*0\r\n").unwrap();
        assert_eq!(val, RespValue::Array(Some(vec![])));
    }

    #[test]
    fn serialize_array() {
        let val = RespValue::array(vec![RespValue::bulk("a"), RespValue::int(1)]);
        let bytes = val.serialize();
        assert_eq!(bytes, b"*2\r\n$1\r\na\r\n:1\r\n");
    }

    #[test]
    fn serialize_null_array() {
        assert_eq!(RespValue::Array(None).serialize(), b"*-1\r\n");
    }

    #[test]
    fn roundtrip_array() {
        roundtrip(&RespValue::array(vec![
            RespValue::bulk("SET"),
            RespValue::bulk("key"),
            RespValue::bulk("value"),
        ]));
        roundtrip(&RespValue::Array(None));
        roundtrip(&RespValue::array(vec![]));
    }

    #[test]
    fn nested_arrays() {
        let inner = RespValue::array(vec![RespValue::int(1), RespValue::int(2)]);
        let outer = RespValue::array(vec![inner.clone(), RespValue::bulk("end")]);
        roundtrip(&outer);
    }

    #[test]
    fn deeply_nested_arrays() {
        let mut val = RespValue::int(42);
        for _ in 0..10 {
            val = RespValue::array(vec![val]);
        }
        roundtrip(&val);
    }

    // -- Mixed types in arrays --

    #[test]
    fn mixed_type_array() {
        let val = RespValue::array(vec![
            RespValue::SimpleString("OK".into()),
            RespValue::err("bad"),
            RespValue::int(42),
            RespValue::bulk("hello"),
            RespValue::null(),
        ]);
        roundtrip(&val);
    }

    // -- Error cases --

    #[test]
    fn empty_input() {
        assert!(parse(b"").is_err());
    }

    #[test]
    fn malformed_line() {
        assert!(parse(b"x\r\n").is_err());
    }

    #[test]
    fn invalid_integer() {
        assert!(parse(b":notanumber\r\n").is_err());
    }

    #[test]
    fn truncated_bulk_string() {
        // Says length 10 but only provides 3 bytes.
        assert!(parse(b"$10\r\nfoo\r\n").is_err());
    }

    // -- Helpers --

    #[test]
    fn helper_constructors() {
        assert_eq!(RespValue::ok(), RespValue::SimpleString("OK".into()));
        assert_eq!(RespValue::null(), RespValue::BulkString(None));
        assert_eq!(RespValue::int(5), RespValue::Integer(5));
        assert_eq!(RespValue::err("fail"), RespValue::Error("ERR fail".into()));
        assert_eq!(
            RespValue::array(vec![RespValue::int(1)]),
            RespValue::Array(Some(vec![RespValue::Integer(1)]))
        );
    }

    // -- Multiple values in sequence --

    #[test]
    fn parse_multiple_values_from_stream() {
        let input = b"+OK\r\n:42\r\n$5\r\nhello\r\n";
        let mut reader = BufReader::new(&input[..]);

        let v1 = parse_resp(&mut reader).unwrap();
        assert_eq!(v1, RespValue::SimpleString("OK".into()));

        let v2 = parse_resp(&mut reader).unwrap();
        assert_eq!(v2, RespValue::Integer(42));

        let v3 = parse_resp(&mut reader).unwrap();
        assert_eq!(v3, RespValue::bulk("hello"));
    }

    // -- Pre-auth allocation / recursion bounds (DoS guards) --

    #[test]
    fn rejects_oversized_bulk_length() {
        // `$2000000000\r\n` would have allocated ~2 GB before this cap. The
        // guard fires on the header line, before any data is read/allocated.
        let frame = format!("${}\r\n", MAX_BULK_LEN + 1);
        let err = parse(frame.as_bytes()).unwrap_err();
        assert!(err.contains("bulk string too large"), "got: {err}");
    }

    #[test]
    fn rejects_oversized_array_count() {
        let frame = format!("*{}\r\n", MAX_ARRAY_COUNT + 1);
        let err = parse(frame.as_bytes()).unwrap_err();
        assert!(err.contains("array too large"), "got: {err}");
    }

    #[test]
    fn rejects_deeply_nested_arrays() {
        // `*1\r\n` repeated past MAX_DEPTH would recurse the stack to
        // exhaustion without the depth guard.
        let mut input = Vec::new();
        for _ in 0..(MAX_DEPTH + 2) {
            input.extend_from_slice(b"*1\r\n");
        }
        let err = parse(&input).unwrap_err();
        assert!(err.contains("nesting too deep"), "got: {err}");
    }

    #[test]
    fn rejects_unbounded_header_line() {
        // An infinite stream that never sends `\n` must not grow `line`
        // without bound. The `take(MAX_LINE_LEN)` cap makes read_line stop
        // and the frame is rejected as malformed. WITHOUT the cap this
        // test hangs / OOMs — that's the regression it guards.
        let mut reader = BufReader::new(std::io::repeat(b'a'));
        let err = parse_resp(&mut reader).unwrap_err();
        assert!(err.contains("Malformed RESP line"), "got: {err}");
    }
}
