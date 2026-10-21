//! The `keymap` section (SDK.md §3.10, SPEC §10.2): `ParseKeymap` and
//! `Format`, `Resolve` and `Lookup`, `Selects` and `Binds`, and an
//! element's keymap's `Program`, `Prefix` and `Scroll`.

mod common;

use hotty::{Mod, Platform, TERMINAL_KEYS, parse_keymap, resolve};
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
        if v.get("lookup").is_none() {
            // An element's keymap outside a text field: its data-keys
            // values joined with a space, the root's first, and no default.
            let m = parse_keymap(&p, &strs(&v["keys"]).join(" "));
            for (key, want) in each(&v["program"]) {
                assert_eq!(Some(m.program(key)), want.as_bool(), "program({key:?})");
            }
            for (key, want) in each(&v["prefix"]) {
                assert_eq!(Some(m.prefix(key)), want.as_bool(), "prefix({key:?})");
            }
            for (key, want) in each(&v["scroll"]) {
                assert_eq!(m.scroll(key), want.as_str(), "scroll({key:?})");
            }
            return;
        }
        let mut values = if terminal {
            vec![TERMINAL_KEYS]
        } else {
            vec![]
        };
        values.extend(strs(&v["keys"]));
        let m = resolve(&p, v["multiline"].as_bool().unwrap(), values);
        for (key, want) in each(&v["lookup"]) {
            assert_eq!(m.lookup(key), want.as_str(), "lookup({key:?})");
        }
        for (key, want) in each(&v["selects"]) {
            assert_eq!(Some(m.selects(key)), want.as_bool(), "selects({key:?})");
        }
        for (key, want) in each(&v["binds"]) {
            assert_eq!(Some(m.binds(key)), want.as_bool(), "binds({key:?})");
        }
    });
}
