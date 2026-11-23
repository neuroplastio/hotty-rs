//! The `keymap` section (SDK.md §3.10, SPEC §10.2): `ParseKeymap` and
//! `Format`, `Resolve` and `Lookup`, `Selects` and `Binds`, and an
//! element's keymap's `Program`, `Prefix` and `Scroll`.

mod common;

use hotty::{Mod, Platform, Reader, TERMINAL_KEYS, parse_keymap, resolve};
use serde_json::Value;

/// The vector's Platform: `ctrl` with the default edit shortcuts where
/// absent (conformance/README.md).
fn platform(v: &Value) -> Platform {
    let p = &v["platform"];
    Platform {
        modifier: match p["mod"].as_str() {
            Some("super") => Mod::Super,
            None | Some("ctrl") => Mod::Ctrl,
            Some(other) => panic!("mod {other}"),
        },
        edit: p["edit"].as_object().map(|e| {
            e.iter()
                .map(|(a, keys)| {
                    let keys = keys.as_array().unwrap();
                    let keys = keys.iter().map(|k| k.as_str().unwrap().to_string());
                    (a.clone(), keys.collect())
                })
                .collect()
        }),
    }
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap()).collect())
        .unwrap_or_default()
}

/// Each key of an object, with its expected value.
fn each(v: &Value) -> impl Iterator<Item = (&String, &Value)> {
    v.as_object().into_iter().flatten()
}

#[test]
fn keymap() {
    let cases = common::section("keymap");
    common::run_all("keymap", &cases, |v| {
        let p = platform(v);
        let terminal = v["terminal_keys"].as_bool() == Some(true);
        if let Some(format) = v.get("format") {
            let value = v["parse"].as_str().unwrap_or_else(|| {
                assert!(
                    terminal,
                    "a format vector with neither parse nor terminal_keys"
                );
                TERMINAL_KEYS
            });
            assert_eq!(parse_keymap(&p, value).format(), format.as_str().unwrap());
            return;
        }
        let reader = match v["reader"].as_str() {
            Some("input") => Reader::Input,
            Some("multiline") => Reader::Multiline,
            Some("button") => Reader::Button,
            Some("select") => Reader::Select,
            Some("date") => Reader::Date,
            Some("element") => Reader::Element,
            Some("selection") => Reader::Selection,
            _ => panic!("unknown reader: {}", v["reader"]),
        };
        let own = v["own"].as_str().unwrap_or_default();
        let inner = terminal
            .then_some(TERMINAL_KEYS)
            .into_iter()
            .chain(strs(&v["inner"]));
        let m = resolve(&p, reader, own, inner);
        for (key, want) in each(&v["lookup"]) {
            assert_eq!(m.lookup(key), want.as_str(), "lookup({key:?})");
        }
        for (key, want) in each(&v["selects"]) {
            assert_eq!(Some(m.selects(key)), want.as_bool(), "selects({key:?})");
        }
        for (key, want) in each(&v["binds"]) {
            assert_eq!(Some(m.binds(key)), want.as_bool(), "binds({key:?})");
        }
        for (key, want) in each(&v["program"]) {
            assert_eq!(Some(m.program(key)), want.as_bool(), "program({key:?})");
        }
        for (key, want) in each(&v["prefix"]) {
            assert_eq!(Some(m.prefix(key)), want.as_bool(), "prefix({key:?})");
        }
        for (key, want) in each(&v["scroll"]) {
            assert_eq!(m.scroll(key), want.as_str(), "scroll({key:?})");
        }
    });
}
