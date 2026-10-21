//! The `encode` vectors: Encode's cleaning, chunks and compression (SDK.md §3.3).

mod common;
mod wire;

use hotty::{Control, encode};
use wire::Out;

#[test]
fn encode_vectors() {
    common::run_all("encode", &common::section("encode"), |v| {
        let control: Control = v["control"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kv| (kv[0].as_str().unwrap(), kv[1].as_str().unwrap()))
            .collect();
        let payload = wire::payload_of(v);
        let out = encode(&control, &payload);

        if let Some(bytes) = v["bytes"].as_str() {
            assert_eq!(out, bytes, "bytes");
        }
        if let Some(chunks) = v["chunks"].as_array() {
            let seqs = wire::sequences(&out);
            assert_eq!(seqs.len(), chunks.len(), "chunks");
            for (seq, c) in seqs.iter().zip(chunks) {
                let body = &seq[hotty::PREFIX.len()..seq.len() - hotty::ST.len()];
                let (ctl, b64) = body.split_once(';').unwrap_or((body, ""));
                assert_eq!(ctl, c["control"].as_str().unwrap(), "chunk control");
                assert_eq!(b64.len() as u64, c["len"].as_u64().unwrap(), "chunk length");
            }
        }

        // It decodes back: the control's keys in order, but o and m, and the payload.
        let got = wire::read(out.as_bytes());
        let [Out::Cmd(m)] = &got[..] else {
            panic!("one command, got {got:?}")
        };
        let want: Control = control
            .iter()
            .filter(|(k, _)| *k != "o" && *k != "m")
            .map(|(k, val)| (k, hotty::clean_value(val)))
            .collect();
        assert_eq!(m.control, want, "control decoded");
        assert_eq!(m.payload, payload, "payload decoded");
    });
}
