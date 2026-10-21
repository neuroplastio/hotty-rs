//! The `detect` vectors: the Detector, step by step (SDK.md §3.8).

mod common;
mod wire;

use hotty::{Decoder, Detector, Fed, State};
use serde_json::Value;

fn state(s: State) -> &'static str {
    match s {
        State::Detecting => "detecting",
        State::Native => "native",
        State::Text => "text",
    }
}

#[test]
fn detect_vectors() {
    common::run_all("detect", &common::section("detect"), |v| {
        let mut d =
            Detector::new(v["n"].as_u64().unwrap_or(1)).late(v["late"].as_bool() == Some(true));
        for (i, step) in v["steps"].as_array().unwrap().iter().enumerate() {
            let at = step["at"].as_u64().unwrap();
            let mut took = None;
            if step["start"].as_bool() == Some(true) {
                let q = d.start(at);
                assert!(q.ends_with("\x1b[c"), "the query is fenced by DA1");
            } else if step["da1"].as_bool() == Some(true) {
                took = Some(d.da1(at));
            } else if let Some(seq) = step["osc"].as_str() {
                let Fed::Complete(m) = Decoder::new().feed(seq.as_bytes()) else {
                    panic!("step {i}: the osc does not decode")
                };
                took = Some(m.reply().is_some_and(|r| d.reply(&r, at)));
            } else if step["tick"].as_bool() == Some(true) {
                d.tick(at);
            } else if step["end"].as_bool() == Some(true) {
                d.end(at);
            } else {
                panic!("step {i}: what kind? {step}");
            }
            if let Some(w) = step["took"].as_bool() {
                assert_eq!(took, Some(w), "step {i}: took");
            }
            if let Some(w) = step["state"].as_str() {
                assert_eq!(state(d.state()), w, "step {i}: state");
            }
            if let Some(w) = step["decided"].as_bool() {
                assert_eq!(d.decided(), w, "step {i}: decided");
            }
            if let Some(w) = step["done"].as_bool() {
                assert_eq!(d.done(), w, "step {i}: done");
            }
            if let Some(w) = step.get("deadline") {
                assert_eq!(d.deadline(), w.as_u64(), "step {i}: deadline");
            }
        }
        if let Some(caps) = v.get("caps") {
            wire::check_caps(caps, d.caps());
        }
    });
}
