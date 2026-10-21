//! What a host writes to a program: replies and events (SPEC §3.6, §9).
//!
//! Each body is one msgpack map, each field of the type the spec's tables
//! give it: an int as an int, a float as a float even when it is whole
//! (`scale: 2.0`), and a field the host has nothing for left out, never
//! sent as nil. Nothing is written that a body may not hold (SPEC §3.3): an
//! int is held within 2^53 − 1 of zero. Never compressed.

use crate::control::Control;
use crate::encode::encode_plain;
use crate::messages::{Area, Caps, Error, Event, Message};
use crate::msgpack::Writer;
use crate::{VERSION, code, kind};

use super::read::speaks;

const MAX_INT: i64 = (1 << 53) - 1;

fn int(w: &mut Writer, v: i64) {
    w.int(v.clamp(-MAX_INT, MAX_INT));
}

// A reply's control: the action, the command's `n` and `s`, `re` naming
// the action answered, then what the reply adds.
fn reply_control(a: &str, cmd: &Message, extra: &[(&str, &str)]) -> Control {
    let mut c = Control::from([("a", a)]);
    for key in ["n", "s"] {
        if let Some(v) = cmd.get(key) {
            c.set(key, v);
        }
    }
    c.set("re", cmd.action());
    for (k, v) in extra {
        c.set(*k, *v);
    }
    c
}

/// A host's ok to `cmd` (SPEC §3.6): its `n` and `s` echoed, `re` naming its
/// action, the keys `extra` adds (a placement's `c` and `r`), and `body`, if
/// any.
pub fn reply_ok(cmd: &Message, extra: &[(&str, &str)], body: &[u8]) -> String {
    encode_plain(&reply_control("ok", cmd, extra), body)
}

/// A host's error reply to `cmd` (SPEC §3.6): its code and detail as the
/// body, and `i`, the index of the op at fault, when [`Error::op`] has one
/// (SPEC §6.1).
pub fn reply_err(cmd: &Message, err: &Error) -> String {
    let i = err.op.map(|i| i.to_string());
    let extra: Vec<(&str, &str)> = i.iter().map(|i| ("i", i.as_str())).collect();
    encode_plain(
        &reply_control("err", cmd, &extra),
        &error_body(&err.code, &err.detail),
    )
}

/// The body of an error reply: `{"code": …, "detail": …}`.
pub fn error_body(code: &str, detail: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.map(2);
    w.kv_str("code", code);
    w.kv_str("detail", detail);
    w.finish()
}

/// A host's answer to a query (SPEC §4), for a host that speaks only
/// [`VERSION`]: its capabilities when the query lists it ([`speaks`]), and
/// `EVERSION` naming it otherwise. `caps.v` should be `VERSION`.
pub fn reply_query(query: &Message, caps: &Caps) -> String {
    if !speaks(query) {
        return reply_err(
            query,
            &Error {
                code: code::EVERSION.to_string(),
                detail: VERSION.to_string(),
                ..Error::default()
            },
        );
    }
    reply_caps(query, caps)
}

/// The capabilities, as the body of the ok to a query.
pub fn reply_caps(query: &Message, caps: &Caps) -> String {
    reply_ok(query, &[], &caps_body(caps))
}

/// The capabilities as a body (SPEC §4): each field they have, in the
/// spec's order. A `caps` event's capabilities that changed write a field
/// that became absent as its type's empty value (SPEC §9): an empty str or
/// list, and a `cell_css` of zero as `{}`.
pub fn caps_body(caps: &Caps) -> Vec<u8> {
    let mut w = Writer::new();
    write_caps(&mut w, caps);
    w.finish()
}

fn strs(w: &mut Writer, v: &[String]) {
    w.array(v.len());
    for s in v {
        w.str(s);
    }
}

fn write_caps(w: &mut Writer, c: &Caps) {
    let n = [
        c.v.is_some(),
        c.cell_css.is_some(),
        c.scale.is_some(),
        c.scheme.is_some(),
        c.modifier.is_some(),
        c.features.is_some(),
        c.limits.is_some(),
        c.net.is_some(),
        c.edit.is_some(),
        c.host.is_some(),
        c.version.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    w.map(n);
    if let Some(v) = &c.v {
        w.kv_str("v", v);
    }
    if let Some(cell) = c.cell_css {
        w.str("cellcss");
        if cell.w == 0.0 && cell.h == 0.0 {
            w.map(0);
        } else {
            w.map(2);
            w.str("w");
            w.float(cell.w);
            w.str("h");
            w.float(cell.h);
        }
    }
    if let Some(s) = c.scale {
        w.str("scale");
        w.float(s);
    }
    if let Some(s) = &c.scheme {
        w.kv_str("scheme", s);
    }
    if let Some(m) = &c.modifier {
        w.kv_str("mod", m);
    }
    if let Some(f) = &c.features {
        w.str("features");
        strs(w, f);
    }
    if let Some(l) = &c.limits {
        w.str("limits");
        w.map(l.len());
        for (k, v) in l {
            w.str(k);
            int(w, *v);
        }
    }
    if let Some(net) = &c.net {
        w.str("net");
        w.map(net.len());
        for (k, v) in net {
            w.str(k);
            strs(w, v);
        }
    }
    if let Some(edit) = &c.edit {
        w.str("edit");
        w.map(edit.len());
        for (k, v) in edit {
            w.str(k);
            strs(w, v);
        }
    }
    if let Some(h) = &c.host {
        w.kv_str("host", h);
    }
    if let Some(v) = &c.version {
        w.kv_str("version", v);
    }
}

fn area(w: &mut Writer, a: &Area) {
    w.map(4);
    for (k, v) in [("c", a.c), ("r", a.r), ("w", a.w), ("h", a.h)] {
        w.str(k);
        int(w, v);
    }
}

// A map of the fields that are there, each written by its function.
type Field<'a> = (&'static str, Box<dyn Fn(&mut Writer) + 'a>);

fn fields(w: &mut Writer, f: Vec<Option<Field<'_>>>) {
    let f: Vec<Field<'_>> = f.into_iter().flatten().collect();
    w.map(f.len());
    for (k, write) in f {
        w.str(k);
        write(w);
    }
}

fn some<'a>(k: &'static str, f: impl Fn(&mut Writer) + 'a) -> Option<Field<'a>> {
    Some((k, Box::new(f)))
}

/// What `e` carries for its kind, as its detail (SPEC §9); `None` for none.
fn detail_of(e: &Event) -> Option<Vec<u8>> {
    let mut w = Writer::new();
    let mods = (e.mods != 0).then_some(i64::from(e.mods));
    let str_field = |k: &'static str, v: &Option<String>| {
        v.clone()
            .and_then(move |v| some(k, move |w: &mut Writer| w.str(&v)))
    };
    let int_field = |k: &'static str, v: Option<i64>| v.and_then(|v| some(k, move |w| int(w, v)));
    match e.kind.as_str() {
        kind::CLICK => {
            let f = vec![
                e.area.and_then(|a| some("area", move |w| area(w, &a))),
                int_field("mods", mods),
                e.link
                    .as_ref()
                    .and_then(|l| str_field("href", &Some(l.href.clone()))),
                e.link.as_ref().and_then(|l| str_field("url", &l.url)),
                str_field("value", &e.value),
                int_field("count", e.count.filter(|&n| n > 1).map(i64::from)),
            ];
            if f.iter().all(Option::is_none) {
                return None;
            }
            fields(&mut w, f);
        }
        kind::PRESS => {
            if e.area.is_none() && mods.is_none() {
                return None;
            }
            fields(
                &mut w,
                vec![
                    e.area.and_then(|a| some("area", move |w| area(w, &a))),
                    int_field("mods", mods),
                ],
            );
        }
        kind::CHANGE => fields(
            &mut w,
            vec![
                e.checked.and_then(|c| some("checked", move |w| w.bool(c))),
                str_field("value", &e.value),
            ],
        ),
        kind::INPUT => fields(&mut w, vec![str_field("value", &e.value)]),
        kind::SUBMIT => {
            // A submit always has its fields, none included (SPEC §9).
            let pairs = e.fields.clone().unwrap_or_default();
            w.map(1);
            w.str("fields");
            w.array(pairs.len());
            for (name, value) in &pairs {
                w.array(2);
                w.str(name);
                w.str(value);
            }
        }
        kind::FIT => {
            let r = e.fit_rows?;
            fields(&mut w, vec![int_field("r", Some(i64::from(r)))]);
        }
        kind::DRAG_START | kind::DRAG | kind::DRAG_END => {
            let d = e.drag?;
            fields(
                &mut w,
                vec![
                    int_field("c", Some(d.c)),
                    int_field("r", Some(d.r)),
                    int_field("mods", mods),
                    int_field("x", d.x),
                    int_field("y", d.y),
                    int_field("tx", d.tx),
                    int_field("ty", d.ty),
                ],
            );
        }
        kind::HOVER => {
            let h = e.hover?;
            if h.out {
                fields(&mut w, vec![some("out", |w| w.bool(true))]);
            } else {
                fields(&mut w, vec![int_field("c", h.c), int_field("r", h.r)]);
            }
        }
        kind::VIEW => {
            let v = e.view.as_ref()?;
            w.map(2);
            w.str("in");
            w.map(v.areas.len());
            for (id, a) in &v.areas {
                w.str(id);
                area(&mut w, a);
            }
            w.str("end");
            w.bool(v.at_end);
        }
        kind::COMMAND => {
            let c = e.command.as_ref()?;
            fields(
                &mut w,
                vec![
                    some("name", |w| w.str(&c.name)),
                    (!c.key.is_empty())
                        .then(|| some("key", |w| w.str(&c.key)))
                        .flatten(),
                    c.has_text
                        .then(|| some("text", |w| w.str(&c.text)))
                        .flatten(),
                ],
            );
        }
        kind::CAPS => write_caps(&mut w, e.changed.as_ref()?),
        _ => return None,
    }
    Some(w.finish())
}

/// An event, as a host writes it to a program (SPEC §9), or a relay to the
/// program behind it, its surface named as that program knows it: `s`,
/// `e` and `t` (left out when empty), and its detail. The detail is the
/// event's payload when it has one, as it came, so a relay passes on what
/// it does not read; otherwise what the event carries for its kind, each
/// field as its type. A `command` or a `caps` event names no surface.
pub fn encode_event(e: &Event) -> String {
    let mut c = Control::from([("a", "ev")]);
    if !matches!(e.kind.as_str(), kind::COMMAND | kind::CAPS)
        && let Some(s) = &e.surface
    {
        c.set("s", s.as_str());
    }
    c.set("e", e.kind.as_str());
    if !e.target.is_empty() {
        c.set("t", e.target.as_str());
    }
    if !e.message.payload.is_empty() {
        return encode_plain(&c, &e.message.payload);
    }
    encode_plain(&c, &detail_of(e).unwrap_or_default())
}
