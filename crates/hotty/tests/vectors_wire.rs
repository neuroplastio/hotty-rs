//! The `wire` vectors: a stream of commands, cut into sequences and decoded
//! (conformance/README.md, "Wire vectors").

mod common;

use hotty::{Decoder, Fed};
use serde_json::Value;

// The stream's OSC sequences, each from ESC ] to its terminator (ST or BEL),
// and what is left of one with none.
fn sequences(stream: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = stream[i..].windows(2).position(|w| w == b"\x1b]") {
        let start = i + at;
        let mut end = start + 2;
        while end < stream.len() {
            if stream[end] == 0x07 {
                end += 1;
                break;
            }
            if stream[end] == 0x1b && stream.get(end + 1) == Some(&b'\\') {
                end += 2;
                break;
            }
            end += 1;
        }
        out.push(&stream[start..end]);
        i = end;
    }
    out
}

fn run(v: &Value) {
    let stream = common::bytes(&v["stream"]);
    let mut d = Decoder::new();
    let mut got = Vec::new();
    for seq in sequences(&stream) {
        if let Fed::Complete(m) = d.feed(seq) {
            got.push(m);
        }
    }
    // SDK.md §3.6 allows a decoder built without zlib to reject compressed bodies.
    let (want, invalid) = if !cfg!(feature = "zlib") && common::name(v) == "o=z payloads are zlib" {
        (&[][..], 1)
    } else {
        (
            v["commands"].as_array().unwrap().as_slice(),
            v["invalid"].as_u64().unwrap(),
        )
    };
    assert_eq!(got.len(), want.len(), "commands");
    for (m, w) in got.iter().zip(want) {
        assert!(
            !m.control.has("m") && !m.control.has("o"),
            "m or o kept: {:?}",
            m.control
        );
        for (k, val) in w["control"].as_object().unwrap() {
            assert_eq!(m.get(k), val.as_str(), "control {k}");
        }
        let text = std::str::from_utf8(&m.payload).expect("the payload is UTF-8");
        assert_eq!(text, w["payload"].as_str().unwrap_or(""), "payload");
    }
    assert_eq!(d.invalid() as u64, invalid, "invalid");
}

#[test]
fn wire() {
    common::run_all("wire", &common::section("wire"), run);
}
