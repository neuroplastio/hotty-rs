//! The conformance vectors (hotty's conformance/vectors.json), shared by
//! the vector runners: one test file for each section (SDK.md §5).
#![allow(dead_code)]

use serde_json::Value;
use std::sync::OnceLock;

/// What this SDK implements, for vectors that name it in `requires`
/// (conformance/README.md): every SDK requirement.
pub const IMPLEMENTS: &[&str] = &[
    "options.unordered",
    "decode.abort-count",
    "decode.unterminated",
    "decode.max",
    "scanner.da1",
    "graphemes",
];

/// The vectors file, parsed once.
pub fn vectors() -> &'static Value {
    static V: OnceLock<Value> = OnceLock::new();
    V.get_or_init(|| {
        let text = include_str!("../testdata/vectors.json");
        serde_json::from_str(text).expect("vectors.json parses")
    })
}

/// A section's vectors that this SDK runs: those whose `requires` it
/// implements. Panics when the section is missing or empty.
pub fn section(name: &str) -> Vec<&'static Value> {
    let all = vectors()[name]
        .as_array()
        .unwrap_or_else(|| panic!("no section {name}"));
    assert!(!all.is_empty(), "section {name} is empty");
    all.iter().filter(|v| applies(v)).collect()
}

/// Whether a vector applies: every name in its `requires` is implemented.
pub fn applies(v: &Value) -> bool {
    match &v["requires"] {
        Value::Null => true,
        Value::String(s) => IMPLEMENTS.contains(&s.as_str()),
        Value::Array(a) => a
            .iter()
            .all(|r| r.as_str().is_some_and(|s| IMPLEMENTS.contains(&s))),
        other => panic!("requires: {other}"),
    }
}

/// A vector's name, for failure messages.
pub fn name(v: &Value) -> &str {
    v["name"].as_str().unwrap_or("?")
}

/// A JSON string's bytes: its text as UTF-8 (conformance/README.md).
pub fn bytes(v: &Value) -> Vec<u8> {
    v.as_str().expect("a string").as_bytes().to_vec()
}

/// Runs each case, catching panics, and fails once at the end with every
/// failing vector's name and message, so one run shows them all.
pub fn run_all(section_name: &str, cases: &[&'static Value], f: impl Fn(&'static Value)) {
    let mut failed = Vec::new();
    for v in cases {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(v)));
        if let Err(e) = r {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            failed.push(format!("  {}: {}", name(v), msg));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} {section_name} vectors failed:\n{}",
        failed.len(),
        cases.len(),
        failed.join("\n")
    );
}
