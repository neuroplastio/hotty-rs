//! OSC sequences into messages, one at a time, joining chunked ones
//! (SDK.md §3.6).

use crate::control::Control;
use crate::messages::Message;
use crate::{DEFAULT_MAX, NUMBER, base64};

/// What [`Decoder::feed`] made of one sequence.
#[derive(Clone, Debug, PartialEq)]
pub enum Fed {
    /// Another OSC, or another number (`ESC ] 72790`): the caller handles
    /// it. A chunked message in progress is not affected.
    NotHotty,
    /// A chunk of a message that is not complete yet.
    Partial,
    /// A whole message.
    Complete(Message),
    /// A malformed message (SPEC §3.7), dropped.
    Invalid,
}

/// Turns OSC sequences into messages, one at a time, joining chunked ones
/// (SPEC §3.4, SDK.md §3.6). A new Decoder is ready to use.
///
/// ```
/// use hotty::{Decoder, Fed};
///
/// let mut d = Decoder::new();
/// let Fed::Complete(m) = d.feed(b"\x1b]7279;a=ok:n=2:re=place:c=40:r=3\x1b\\") else {
///     panic!("not a message");
/// };
/// assert_eq!(m.reply().unwrap().rows, Some(3));
/// ```
#[derive(Debug, Default)]
pub struct Decoder {
    /// The most bytes one message decodes to, after base64 and zlib; 0 is
    /// [`DEFAULT_MAX`]. Past it the message is malformed, counted once, and
    /// its further chunks are dropped up to its last (SPEC §3.4). A host
    /// that decodes with a Decoder reports its max as `limits.message`
    /// (SPEC §13).
    pub max: usize,
    invalid: usize,
    pending: Option<Pending>,
}

// A chunked message in progress: its control and its base64 so far, or,
// once it passed max, nothing, its chunks dropped as they come.
#[derive(Debug)]
struct Pending {
    control: Control,
    b64: Vec<u8>,
    over: bool,
}

// The most base64 a message of max bytes is sent as: zlib may make it a
// little longer than it decodes to.
fn encoded_max(max: usize) -> usize {
    (max + max / 256 + 64).div_ceil(3) * 4
}

// A key, as SPEC §3.2 has it: a letter, then letters, digits, - and _.
fn valid_key(k: &[u8]) -> bool {
    k.first().is_some_and(u8::is_ascii_alphabetic)
        && k.iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

// A value: printable ASCII but : ; and =.
fn valid_value(v: &[u8]) -> bool {
    v.iter()
        .all(|&c| (0x20..=0x7e).contains(&c) && c != b':' && c != b';' && c != b'=')
}

/// The control's pairs, strictly as SPEC §3.2 has them: a key that does not
/// parse, a value outside printable ASCII, a key given twice, or an empty
/// control, and it is `None`.
fn parse_control(text: &[u8]) -> Option<Control> {
    if text.is_empty() {
        return None;
    }
    let mut c = Control::new();
    for pair in text.split(|&b| b == b':') {
        let eq = pair.iter().position(|&b| b == b'=')?;
        let (k, v) = (&pair[..eq], &pair[eq + 1..]);
        if !valid_key(k) || !valid_value(v) {
            return None;
        }
        // Both are ASCII, checked above.
        let (k, v) = (std::str::from_utf8(k).ok()?, std::str::from_utf8(v).ok()?);
        if c.has(k) {
            return None;
        }
        c.push(k, v);
    }
    Some(c)
}

impl Decoder {
    /// A Decoder with the default `max`.
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// A Decoder that decodes no message to more than `max` bytes.
    pub fn with_max(max: usize) -> Decoder {
        Decoder {
            max,
            ..Decoder::default()
        }
    }

    /// The malformed messages dropped so far, aborted chunked ones included
    /// (SPEC §3.7).
    pub fn invalid(&self) -> usize {
        self.invalid
    }

    fn limit(&self) -> usize {
        if self.max > 0 { self.max } else { DEFAULT_MAX }
    }

    /// Takes one OSC sequence, `ESC ] …`, with or without its terminator
    /// (ST or BEL; an environment's input parser may strip it, SDK.md §4.2).
    pub fn feed(&mut self, seq: &[u8]) -> Fed {
        let Some(rest) = seq
            .strip_prefix(b"\x1b]")
            .and_then(|s| s.strip_prefix(NUMBER.as_bytes()))
        else {
            return Fed::NotHotty;
        };
        if rest.first().is_some_and(u8::is_ascii_digit) {
            return Fed::NotHotty;
        }
        let rest = rest
            .strip_suffix(b"\x1b\\")
            .or_else(|| rest.strip_suffix(b"\x07"))
            .unwrap_or(rest);
        let Some(text) = rest.strip_prefix(b";") else {
            return self.bad();
        };
        if !text.is_ascii() {
            return self.bad();
        }
        let (ctl, b64) = match text.iter().position(|&b| b == b';') {
            Some(i) => (&text[..i], &text[i + 1..]),
            None => (text, &text[text.len()..]),
        };
        let Some(control) = parse_control(ctl) else {
            return self.bad();
        };

        if control.len() == 1 && control.has("m") {
            return self.continuation(control.get("m") == Some("1"), control.get("m"), b64);
        }
        if let Some(p) = self.pending.take() {
            // Another message before the last chunk aborts the one in progress.
            if !p.over {
                self.invalid += 1;
            }
        }
        match control.get("m") {
            Some("1") => {
                let over = b64.len() > encoded_max(self.limit());
                if over {
                    self.invalid += 1;
                }
                self.pending = Some(Pending {
                    control,
                    b64: if over { Vec::new() } else { b64.to_vec() },
                    over,
                });
                Fed::Partial
            }
            None | Some("0") => self.finish(control, b64),
            Some(_) => self.bad(),
        }
    }

    fn continuation(&mut self, more: bool, m: Option<&str>, b64: &[u8]) -> Fed {
        if !matches!(m, Some("0" | "1")) || self.pending.is_none() {
            return self.bad();
        }
        let limit = encoded_max(self.limit());
        let p = self.pending.as_mut().expect("a message in progress");
        if !p.over && p.b64.len() + b64.len() > limit {
            // Too long: counted once, and its chunks dropped as they come.
            p.b64 = Vec::new();
            p.over = true;
            self.invalid += 1;
        }
        if !p.over {
            p.b64.extend_from_slice(b64);
        }
        if more {
            return Fed::Partial;
        }
        let p = self.pending.take().expect("a message in progress");
        if p.over {
            return Fed::Invalid;
        }
        self.finish(p.control, &p.b64)
    }

    // A malformed message: it aborts a chunked one in progress, which makes
    // two (SPEC §3.7).
    fn bad(&mut self) -> Fed {
        if let Some(p) = self.pending.take()
            && !p.over
        {
            self.invalid += 1;
        }
        self.invalid += 1;
        Fed::Invalid
    }

    // A payload that decodes to more than max bytes is refused, and
    // inflating stops past it.
    fn finish(&mut self, mut control: Control, b64: &[u8]) -> Fed {
        let max = self.limit();
        let mut payload = if b64.len() > encoded_max(max) {
            None
        } else {
            base64::decode(b64)
        };
        if control.get("o") == Some("z") {
            payload = payload.and_then(|p| inflate(&p, max));
        }
        match payload {
            Some(p) if p.len() <= max => {
                control.remove("m");
                control.remove("o");
                Fed::Complete(Message::new(control, p))
            }
            _ => {
                self.invalid += 1;
                Fed::Invalid
            }
        }
    }
}

/// zlib's inflation of `data`, stopping one byte past `max`, so that a
/// caller sees it went past.
#[cfg(feature = "zlib")]
fn inflate(data: &[u8], max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// Without zlib, a compressed payload does not decode (SDK.md §3.6).
#[cfg(not(feature = "zlib"))]
fn inflate(_data: &[u8], _max: usize) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_strict() {
        assert!(parse_control(b"a=ok:n=1").is_some());
        assert!(parse_control(b"a=").is_some());
        assert!(parse_control(b"").is_none());
        assert!(parse_control(b"a=1:a=2").is_none());
        assert!(parse_control(b"a").is_none());
        assert!(parse_control(b"1a=x").is_none());
        assert!(parse_control(b"a=b=c").is_none());
        assert!(parse_control(b"a=x::b=y").is_none());
        assert!(parse_control(b"a=\x07").is_none());
    }

    #[test]
    fn a_sequence_needs_its_semicolon() {
        let mut d = Decoder::new();
        assert_eq!(d.feed(b"\x1b]7279\x1b\\"), Fed::Invalid);
        assert_eq!(d.feed(b"\x1b]72790;a=q\x1b\\"), Fed::NotHotty);
        assert_eq!(d.feed(b"\x1b]11;rgb:0/0/0\x07"), Fed::NotHotty);
        assert_eq!(d.invalid(), 1);
    }
}
