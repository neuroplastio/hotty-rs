//! Reading back what the builders and Encode write, for the `build`,
//! `encode` and `detect` runners.
#![allow(dead_code)]

use hotty::{Control, Decoder, Fed, Message, Scanner, Segment};
use serde_json::Value;

/// A piece of a builder's output: bytes that are not hotty's, or one
/// command, its chunks joined and decoded.
#[derive(Debug, PartialEq)]
pub enum Out {
    Raw(Vec<u8>),
    Cmd(Message),
}

/// Cuts output into raw bytes and commands, decoding each command.
pub fn read(out: &[u8]) -> Vec<Out> {
    let mut s = Scanner::new();
    let mut segs = s.feed(out);
    segs.extend(s.flush());
    assert_eq!(s.invalid(), 0, "malformed sequences in {out:?}");
    let mut d = Decoder::new();
    let mut res = Vec::new();
    for seg in segs {
        match seg {
            Segment::Pass(b) => res.push(Out::Raw(b)),
            Segment::Osc(b) => match d.feed(&b) {
                Fed::Complete(m) => res.push(Out::Cmd(m)),
                Fed::Partial => {}
                other => panic!("{other:?} for {:?}", String::from_utf8_lossy(&b)),
            },
            Segment::Da1(_) => unreachable!(),
        }
    }
    res
}

/// The sequences of an output, each from `ESC ]` to its terminator.
pub fn sequences(out: &str) -> Vec<&str> {
    out.split_inclusive("\x1b\\").collect()
}

/// A control as the vectors write it, `{key: value}`, in order.
pub fn control_of(v: &Value) -> Control {
    v.as_object()
        .expect("a control")
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str().expect("a string value")))
        .collect()
}

/// Base64, as the vectors' `payload_b64` has it.
pub fn b64(s: &str) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|&c| c != b'=') {
        let v = A.iter().position(|&a| a == c).expect("base64") as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// A command's expected payload: `payload` as text, or `payload_b64`.
pub fn payload_of(v: &Value) -> Vec<u8> {
    if let Some(b) = v["payload_b64"].as_str() {
        return b64(b);
    }
    v["payload"]
        .as_str()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_default()
}

/// Checks capabilities against the vectors' `caps`: each field it names,
/// `null` for none.
pub fn check_caps(want: &Value, got: Option<&hotty::Caps>) {
    if want.is_null() {
        assert!(got.is_none(), "caps: want none, got {got:?}");
        return;
    }
    let got = got.unwrap_or_else(|| panic!("caps: want {want}, got none"));
    for (k, w) in want.as_object().expect("caps").iter() {
        let g: Value = match k.as_str() {
            "v" => got.v.clone().into(),
            "scheme" => got.scheme.clone().into(),
            "mod" => got.modifier.clone().into(),
            "host" => got.host.clone().into(),
            "version" => got.version.clone().into(),
            "scale" => got.scale.into(),
            "features" => got.features.clone().into(),
            "cell_css" => got
                .cell_css
                .map(|c| serde_json::json!({"w": c.w, "h": c.h}))
                .unwrap_or(Value::Null),
            _ => continue,
        };
        assert!(num_eq(w, &g), "caps.{k}: want {w}, got {g}");
    }
}

/// Values equal, numbers as numbers.
pub fn num_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| num_eq(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| num_eq(v, w)))
        }
        _ => a == b,
    }
}
