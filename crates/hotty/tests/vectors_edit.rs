//! The `edit` section (SDK.md §4.6, SPEC §10.2): a Field's value, caret and
//! selection after each action, in grapheme clusters.

mod common;

use hotty::Field;
use serde_json::Value;

fn count(v: &Value) -> usize {
    v.as_u64().expect("a count") as usize
}

#[test]
fn edit() {
    let cases = common::section("edit");
    common::run_all("edit", &cases, |v| {
        let f = &v["field"];
        let mut field = Field::new(f["value"].as_str().unwrap());
        field.caret = count(&f["caret"]);
        field.anchor = f.get("anchor").map(count);
        field.multiline = f["multiline"].as_bool().unwrap_or(false);
        field.password = f["password"].as_bool().unwrap_or(false);
        field.rows = f.get("rows").map_or(1, count);
        for (i, step) in v["steps"].as_array().unwrap().iter().enumerate() {
            let changed = if let Some(a) = step.get("do") {
                field.do_action(a.as_str().unwrap())
            } else if let Some(a) = step.get("extend") {
                field.extend(a.as_str().unwrap())
            } else if let Some(t) = step.get("type") {
                field.type_text(t.as_str().unwrap())
            } else if let Some(s) = step.get("select") {
                field.select(count(&s[0]), count(&s[1]));
                false
            } else {
                panic!("step {i}: {step}");
            };
            if let Some(want) = step.get("value") {
                assert_eq!(field.value, want.as_str().unwrap(), "step {i}: value");
            }
            if let Some(want) = step.get("caret") {
                assert_eq!(field.caret, count(want), "step {i}: caret");
            }
            if let Some(want) = step.get("anchor") {
                // With nothing selected, the anchor is the caret.
                let anchor = field.anchor.unwrap_or(field.caret);
                assert_eq!(anchor, count(want), "step {i}: anchor");
            }
            if let Some(want) = step.get("changed") {
                assert_eq!(Some(changed), want.as_bool(), "step {i}: changed");
            }
        }
    });
}
