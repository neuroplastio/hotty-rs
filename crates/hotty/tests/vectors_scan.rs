//! The `scan` vectors: the Scanner (SDK.md §3.7), fed whole, cut in two at
//! every offset (or near the ends and at a stride, in a long stream), and a
//! byte at a time.

mod common;

use hotty::{Scanner, Segment};
use serde_json::Value;

fn expected(v: &Value) -> Vec<Segment> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|s| {
                    let (k, t) = s.as_object().unwrap().iter().next().unwrap();
                    let b = common::bytes(t);
                    match k.as_str() {
                        "pass" => Segment::Pass(b),
                        "osc" => Segment::Osc(b),
                        "da1" => Segment::Da1(b),
                        other => panic!("segment {other}"),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

// Adjacent pass segments merged, as the vectors write them.
fn merged(segs: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for s in segs {
        if let (Segment::Pass(b), Some(Segment::Pass(last))) = (&s, out.last_mut()) {
            last.extend_from_slice(b);
            continue;
        }
        out.push(s);
    }
    out
}

struct Run {
    segments: Vec<Segment>,
    held: Vec<u8>,
    flush: Vec<Segment>,
    invalid: usize,
}

fn run(da1: bool, pieces: &[&[u8]]) -> Run {
    let mut s = Scanner::new().da1(da1);
    let mut segments = Vec::new();
    for p in pieces {
        segments.extend(s.feed(p));
    }
    let held = s.holding().to_vec();
    let flush = merged(s.flush());
    Run {
        segments: merged(segments),
        held,
        flush,
        invalid: s.invalid(),
    }
}

// Where to cut a stream in two: every offset, or in a long one, every
// offset near its ends and at a stride between.
fn cuts(n: usize) -> Vec<usize> {
    if n <= 512 {
        return (0..=n).collect();
    }
    let mut c: Vec<usize> = (0..=64).chain(n - 64..=n).collect();
    c.extend((64..n - 64).step_by(97));
    c
}

#[test]
fn scan() {
    common::run_all("scan", &common::section("scan"), |v| {
        let stream = common::bytes(&v["stream"]);
        let da1 = common::applies(v) && v["requires"].to_string().contains("scanner.da1");
        let want_segments = expected(&v["segments"]);
        let want_held = v["held"]
            .as_str()
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();
        let want_flush = expected(&v["flush"]);
        let want_invalid = v["invalid"].as_u64().unwrap_or(0) as usize;

        let check = |how: &str, r: Run| {
            assert_eq!(r.segments, want_segments, "{how}: segments");
            assert_eq!(r.held, want_held, "{how}: held");
            assert_eq!(r.flush, want_flush, "{how}: flush");
            assert_eq!(r.invalid, want_invalid, "{how}: invalid");
        };
        check("whole", run(da1, &[&stream]));
        for at in cuts(stream.len()) {
            check(
                &format!("cut at {at}"),
                run(da1, &[&stream[..at], &stream[at..]]),
            );
        }
        let bytes: Vec<&[u8]> = stream.chunks(1).collect();
        check("byte by byte", run(da1, &bytes));
    });
}

#[test]
fn a_scanner_without_da1_passes_da1_answers() {
    let mut s = Scanner::new();
    let segs = s.feed(b"\x1b[?62;22c");
    assert_eq!(segs, vec![Segment::Pass(b"\x1b[?62;22c".to_vec())]);
}

#[test]
fn in_sequence_tells_a_sequence_from_its_start() {
    let mut s = Scanner::new();
    s.feed(b"\x1b]72");
    assert!(!s.in_sequence());
    s.feed(b"79;a=ok");
    assert!(s.in_sequence());
}
