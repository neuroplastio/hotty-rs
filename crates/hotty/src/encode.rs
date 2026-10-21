//! One command as OSC sequences (SDK.md §3.3).

use std::fmt::Write as _;

use crate::control::Control;
use crate::{CHUNK, NUMBER, base64};

/// `ESC ] 7279 ;`: how every hotty sequence starts.
pub const PREFIX: &str = "\x1b]7279;";
/// The String Terminator, `ESC \`: how every sequence a program sends ends.
pub const ST: &str = "\x1b\\";

#[cfg(feature = "zlib")]
const COMPRESS_FROM: usize = 256;

fn value_char(c: char) -> bool {
    matches!(c, ' '..='~') && !matches!(c, ':' | ';' | '=')
}

/// A control value as it may be sent: each character it may not hold
/// (`:`, `;`, `=`, a control character, anything outside ASCII) becomes
/// one `_`, one for each code point (SPEC §3.2).
pub fn clean_value(v: &str) -> String {
    if v.chars().all(value_char) {
        return v.to_string();
    }
    v.chars()
        .map(|c| if value_char(c) { c } else { '_' })
        .collect()
}

/// One command as one or more OSC sequences (SDK.md §3.3): the control and
/// the payload, base64-encoded and split into chunks of at most 4096
/// bytes; the first carries the control, a further one only `m`. Keys go
/// out in order, each value cleaned ([`clean_value`]), but a given `o` or
/// `m`, which are Encode's own. A payload of 256 bytes or more goes
/// compressed (`o=z`) when that makes it smaller and the crate has zlib
/// (feature `zlib`).
pub fn encode(control: &Control, payload: &[u8]) -> String {
    encode_with(control, payload, true)
}

/// [`encode`] without compression: what a host sends, which it never
/// compresses (SPEC §3.3).
pub fn encode_plain(control: &Control, payload: &[u8]) -> String {
    encode_with(control, payload, false)
}

#[cfg(feature = "zlib")]
fn deflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write as _;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    z.write_all(data).ok()?;
    let packed = z.finish().ok()?;
    (packed.len() < data.len()).then_some(packed)
}

fn encode_with(control: &Control, payload: &[u8], compress: bool) -> String {
    #[cfg(feature = "zlib")]
    let packed = if compress && payload.len() >= COMPRESS_FROM {
        deflate(payload)
    } else {
        None
    };
    #[cfg(not(feature = "zlib"))]
    let packed: Option<Vec<u8>> = {
        let _ = compress;
        None
    };
    let b64 = base64::encode(packed.as_deref().unwrap_or(payload));

    let mut ctl = String::new();
    for (k, v) in control.iter().filter(|(k, _)| *k != "o" && *k != "m") {
        if !ctl.is_empty() {
            ctl.push(':');
        }
        let _ = write!(ctl, "{k}={}", clean_value(v));
    }
    if packed.is_some() {
        ctl.push_str(if ctl.is_empty() { "o=z" } else { ":o=z" });
    }

    let mut out = String::with_capacity(b64.len() + ctl.len() + 16);
    if b64.len() <= CHUNK {
        let _ = write!(out, "\x1b]{NUMBER};{ctl}");
        if !b64.is_empty() {
            out.push(';');
            out.push_str(&b64);
        }
        out.push_str(ST);
        return out;
    }
    let mut start = 0;
    while start < b64.len() {
        let end = (start + CHUNK).min(b64.len());
        let piece = &b64[start..end];
        if start == 0 {
            let sep = if ctl.is_empty() { "" } else { ":" };
            let _ = write!(out, "{PREFIX}{ctl}{sep}m=1;{piece}{ST}");
        } else {
            let more = if end < b64.len() { '1' } else { '0' };
            let _ = write!(out, "{PREFIX}m={more};{piece}{ST}");
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_values_not_keys() {
        assert_eq!(clean_value("caf\u{e9}:\u{1f600}"), "caf___");
        assert_eq!(clean_value("a=b;c\x07"), "a_b_c_");
        let c = Control::from([("a", "focus"), ("t", "x:y")]);
        assert_eq!(encode(&c, b""), "\x1b]7279;a=focus:t=x_y\x1b\\");
    }

    #[test]
    fn o_and_m_are_encodes() {
        let c = Control::from([("a", "res"), ("o", "z"), ("m", "1"), ("id", "x")]);
        assert_eq!(encode(&c, b"hi"), "\x1b]7279;a=res:id=x;aGk=\x1b\\");
    }

    #[test]
    fn chunks_at_4096() {
        let payload: Vec<u8> = (0..3073u32).map(|i| (i * 7919 % 251) as u8).collect();
        let s = encode_plain(&Control::from([("a", "res")]), &payload);
        let seqs: Vec<&str> = s.split(ST).filter(|p| !p.is_empty()).collect();
        assert_eq!(seqs.len(), 2);
        assert!(seqs[0].starts_with("\x1b]7279;a=res:m=1;"));
        assert!(seqs[1].starts_with("\x1b]7279;m=0;"));
    }
}
