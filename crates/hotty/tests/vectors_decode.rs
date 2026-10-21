//! The `decode` vectors: the Decoder and the messages (SDK.md §3.6, §3.9).

mod common;

use hotty::{Caps, Decoder, Event, Fed, Message, Reply};
use serde_json::{Map, Value, json};

// The vectors' numbers compare as numbers, ints and floats alike.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

// Every key `want` lists must match `got`; a nested object is compared by
// its listed keys too, but view's areas, which are compared whole.
fn check(what: &str, got: &Value, want: &Value) {
    let Some(want) = want.as_object() else {
        assert!(same(got, want), "{what}: got {got}, want {want}");
        return;
    };
    let Some(have) = got.as_object() else {
        panic!("{what}: got {got}, want {}", Value::Object(want.clone()));
    };
    for (k, w) in want {
        let g = have.get(k).unwrap_or(&Value::Null);
        let path = format!("{what}.{k}");
        if w.is_object() && k != "areas" && !g.is_null() {
            check(&path, g, w);
        } else {
            assert!(same(g, w), "{path}: got {g}, want {w}");
        }
    }
}

fn opt<T: Into<Value>>(v: Option<T>) -> Value {
    v.map_or(Value::Null, Into::into)
}

fn reply_json(r: &Reply) -> Value {
    json!({
        "ok": r.ok,
        "re": if r.re.is_empty() { Value::Null } else { r.re.clone().into() },
        "n": opt(r.n),
        "surface": opt(r.surface.clone()),
        "cols": opt(r.cols),
        "rows": opt(r.rows),
        "code": opt(r.code.clone()),
        "detail": opt(r.detail.clone()),
        "op": opt(r.op),
    })
}

fn area_json(a: &hotty::Area) -> Value {
    json!({ "c": a.c, "r": a.r, "w": a.w, "h": a.h })
}

fn event_json(e: &Event) -> Value {
    json!({
        "surface": opt(e.surface.clone()),
        "kind": e.kind,
        "target": e.target,
        "value": opt(e.value.clone()),
        "checked": opt(e.checked),
        "fields": opt(e.fields.as_ref().map(|f| {
            f.iter().map(|(n, v)| json!([n, v])).collect::<Vec<_>>()
        })),
        "mods": e.mods,
        "link": opt(e.link.as_ref().map(|l| json!({ "href": l.href, "url": opt(l.url.clone()) }))),
        "fit_rows": opt(e.fit_rows),
        "drag": opt(e.drag.map(|d| json!({
            "c": d.c, "r": d.r, "x": opt(d.x), "y": opt(d.y), "tx": opt(d.tx), "ty": opt(d.ty),
        }))),
        "hover": opt(e.hover.map(|h| json!({ "c": opt(h.c), "r": opt(h.r), "out": h.out }))),
        "area": opt(e.area.as_ref().map(area_json)),
        "count": opt(e.count),
        "view": opt(e.view.as_ref().map(|v| json!({
            "areas": v.areas.iter().map(|(k, a)| (k.clone(), area_json(a))).collect::<Map<_, _>>(),
            "at_end": v.at_end,
        }))),
        "command": opt(e.command().map(|c| json!({
            "name": c.name, "key": c.key, "text": c.text, "has_text": c.has_text,
        }))),
    })
}

fn caps_json(c: &Caps, has: Option<&Value>) -> Value {
    let mut o = json!({
        "v": opt(c.v.clone()),
        "cell_css": opt(c.cell_css.map(|s| json!({ "w": s.w, "h": s.h }))),
        "scale": opt(c.scale),
        "scheme": opt(c.scheme.clone()),
        "mod": opt(c.modifier.clone()),
        "features": opt(c.features.clone()),
        "limits": opt(c.limits.as_ref().map(|l| l.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>())),
        "net": opt(c.net.as_ref().map(|n| n.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>())),
        "edit": opt(c.edit.as_ref().map(|e| e.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>())),
        "host": opt(c.host.clone()),
        "version": opt(c.version.clone()),
        "light": c.light(),
        "fields": sorted(c.fields().iter().map(|f| json!(f)).collect()),
    });
    if let Some(has) = has.and_then(Value::as_object) {
        o["has"] = has
            .keys()
            .map(|f| (f.clone(), json!(c.has(f))))
            .collect::<Map<_, _>>()
            .into();
    }
    o
}

fn sorted(mut v: Vec<Value>) -> Value {
    v.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    Value::Array(v)
}

fn control_json(m: &Message) -> Value {
    m.control
        .iter()
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect::<Map<_, _>>()
        .into()
}

fn result_name(f: &Fed) -> &'static str {
    match f {
        Fed::NotHotty => "not_hotty",
        Fed::Partial => "partial",
        Fed::Complete(_) => "complete",
        Fed::Invalid => "invalid",
    }
}

fn compressed(v: &Value) -> bool {
    v["seqs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("o=z"))
}

fn run(v: &Value) {
    let mut d = Decoder::new();
    if let Some(max) = v["max"].as_u64() {
        d.max = max as usize;
    }
    let mut results = Vec::new();
    let mut messages = Vec::new();
    for s in v["seqs"].as_array().unwrap() {
        let f = d.feed(&common::bytes(s));
        results.push(json!(result_name(&f)));
        if let Fed::Complete(m) = f {
            messages.push(m);
        }
    }
    assert_eq!(Value::Array(results), v["results"], "results");
    assert_eq!(
        d.invalid() as u64,
        v["invalid"].as_u64().unwrap(),
        "invalid"
    );
    let want = v["messages"].as_array().unwrap();
    assert_eq!(messages.len(), want.len(), "messages");
    for (i, (m, w)) in messages.iter().zip(want).enumerate() {
        let at = |what: &str| format!("message {i}: {what}");
        assert!(
            same(&control_json(m), &w["control"]),
            "{}: got {}, want {}",
            at("control"),
            control_json(m),
            w["control"]
        );
        if let Some(want) = w.get("reply") {
            let r = m.reply().unwrap_or_else(|| panic!("{}", at("not a reply")));
            check(&at("reply"), &reply_json(&r), want);
        }
        if let Some(want) = w.get("event") {
            let e = m
                .event()
                .unwrap_or_else(|| panic!("{}", at("not an event")));
            check(&at("event"), &event_json(&e), want);
        }
        if let Some(want) = w.get("changed") {
            let e = m
                .event()
                .unwrap_or_else(|| panic!("{}", at("not an event")));
            let got = e
                .changed()
                .map_or(Value::Null, |c| caps_json(c, want.get("has")));
            let mut want = want.clone();
            if let Some(f) = want.get("fields").and_then(Value::as_array) {
                want["fields"] = sorted(f.clone());
            }
            check(&at("changed"), &got, &want);
        }
        if let Some(want) = w.get("caps") {
            let r = m.reply().unwrap_or_else(|| panic!("{}", at("not a reply")));
            let got = r
                .caps
                .as_ref()
                .map_or(Value::Null, |c| caps_json(c, want.get("has")));
            if want.is_null() {
                assert!(got.is_null(), "{}: got {got}, want none", at("caps"));
            } else {
                check(&at("caps"), &got, want);
            }
        }
    }
}

#[test]
fn decode() {
    let cases: Vec<_> = common::section("decode")
        .into_iter()
        .filter(|v| cfg!(feature = "zlib") || !compressed(v))
        .collect();
    common::run_all("decode", &cases, run);
}
