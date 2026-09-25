//! LEB128 varints and zigzag signed integers.

/// Append `v` as an unsigned LEB128 varint (1 to 10 bytes).
pub fn write_u64(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Bytes `write_u64` uses for `v`.
pub fn len_u64(v: u64) -> usize {
    (64 - (v | 1).leading_zeros() as usize).div_ceil(7)
}

/// Read an unsigned LEB128 varint and advance `bytes` past it. None when
/// it is truncated or longer than 10 bytes.
pub fn read_u64(bytes: &mut &[u8]) -> Option<u64> {
    let mut v: u64 = 0;
    for (i, &b) in bytes.iter().enumerate().take(10) {
        let part = (b & 0x7f) as u64;
        if i == 9 && part > 1 {
            return None; // past 64 bits
        }
        v |= part << (7 * i);
        if b & 0x80 == 0 {
            *bytes = &bytes[i + 1..];
            return Some(v);
        }
    }
    None
}

/// Append a signed integer as a zigzag varint (small magnitudes are short).
pub fn write_i64(out: &mut Vec<u8>, v: i64) {
    write_u64(out, ((v << 1) ^ (v >> 63)) as u64);
}

pub fn read_i64(bytes: &mut &[u8]) -> Option<i64> {
    let z = read_u64(bytes)?;
    Some(((z >> 1) as i64) ^ -((z & 1) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_sizes() {
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            let mut b = Vec::new();
            write_u64(&mut b, v);
            let mut s = &b[..];
            assert_eq!(read_u64(&mut s), Some(v));
            assert!(s.is_empty());
        }
        for v in [0i64, -1, 1, -64, 63, -65, i64::MIN, i64::MAX] {
            let mut b = Vec::new();
            write_i64(&mut b, v);
            let mut s = &b[..];
            assert_eq!(read_i64(&mut s), Some(v));
        }
        let mut b = Vec::new();
        write_i64(&mut b, -63);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn len_matches_the_encoding() {
        for v in [0u64, 1, 127, 128, 16383, 16384, u64::MAX] {
            let mut b = Vec::new();
            write_u64(&mut b, v);
            assert_eq!(len_u64(v), b.len(), "{v}");
        }
    }

    #[test]
    fn truncated_and_overlong_varints_are_refused() {
        assert_eq!(read_u64(&mut &[0x80u8][..]), None);
        assert_eq!(read_u64(&mut &[0xffu8; 11][..]), None);
        assert_eq!(
            read_u64(&mut &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02][..]),
            None
        );
    }
}
