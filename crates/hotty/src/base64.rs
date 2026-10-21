//! Base64, the standard alphabet (SPEC §3.3): padded on the way out; on the
//! way in, with or without padding, whitespace ignored, and any other byte
//! refused.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The base64 of `bytes`, padded.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// How many base64 bytes `n` bytes take, padded.
pub(crate) fn encoded_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

fn value(c: u8) -> Option<u32> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    } as u32)
}

/// The bytes `text` encodes: with or without padding, whitespace ignored.
/// `None` when it holds any other byte, padding anywhere but at the end, or
/// a lone sixth of a byte.
pub(crate) fn decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut pad = 0usize;
    for &c in text {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            pad += 1;
            continue;
        }
        if pad > 0 {
            return None;
        }
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // What is left is 0, 2 or 4 bits of a final quantum: 6 is a lone sixth.
    if bits >= 6 || pad > 2 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for n in 0..70 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let e = encode(&b);
            assert_eq!(e.len(), encoded_len(n));
            assert_eq!(decode(e.as_bytes()).as_deref(), Some(&b[..]));
            assert_eq!(
                decode(e.trim_end_matches('=').as_bytes()).as_deref(),
                Some(&b[..])
            );
        }
    }

    #[test]
    fn reads_leniently_and_refuses_the_rest() {
        assert_eq!(decode(b"aGk=").unwrap(), b"hi");
        assert_eq!(decode(b"aG k\n").unwrap(), b"hi");
        assert_eq!(decode(b"").unwrap(), b"");
        assert!(decode(b"aGk*").is_none());
        assert!(decode(b"a").is_none());
        assert!(decode(b"aG=k").is_none());
        assert!(decode(b"-_").is_none());
    }
}
