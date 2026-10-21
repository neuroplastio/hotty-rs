//! The `keys` section (SDK.md §3.10, SPEC §10.4): key names, the keys in
//! what a terminal sends, and the text a key types.

mod common;

use hotty::{Mod, Platform, decode_keys, key_text, parse_key, read_key};
use serde_json::Value;

#[test]
fn keys() {
    let cases = common::section("keys");
    common::run_all("keys", &cases, |v| {
        if let Some(input) = v.get("input") {
            let got = decode_keys(&common::bytes(input));
            let want: Vec<Option<String>> = v["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().map(str::to_string))
                .collect();
            assert_eq!(got, want, "decode_keys({input})");
        } else if let Some(text) = v.get("text") {
            let key = v["key"].as_str().unwrap();
            assert_eq!(key_text(key).as_deref(), text.as_str(), "key_text({key:?})");
        } else {
            let key = v["key"].as_str().unwrap();
            let got = match v.get("mod").and_then(Value::as_str) {
                Some(m) => {
                    let modifier = match m {
                        "super" => Mod::Super,
                        "ctrl" => Mod::Ctrl,
                        other => panic!("mod {other}"),
                    };
                    read_key(&Platform::new(modifier), key)
                }
                None => parse_key(key),
            };
            assert_eq!(got.as_deref(), v["canon"].as_str(), "{key:?}");
        }
    });
}
