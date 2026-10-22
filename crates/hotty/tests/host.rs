//! The host's side: a program's commands read as a host reads them, a
//! host's replies and events written, and a program's output split. Each
//! round trip goes through the other side, which the conformance vectors
//! check.

mod wire;

use std::collections::BTreeMap;

use hotty::host::{
    ControlKind, Output, Seq, Splitter, control_kind, cursor_below, delta_of, encode_event,
    field_fits, field_of, menu_of, menu_show_of, placement_of, quiet_of, reply_caps, reply_err,
    reply_ok, reply_query, speaks, touch_of,
};
use hotty::keys::{KeyParts, Kind, Mod, Platform, SHIFT, char_key, parse_keymap};
use hotty::*;
use wire::Out;

// The one command a builder's output holds.
fn command(out: impl std::fmt::Display) -> Message {
    let got = wire::read(out.to_string().as_bytes());
    let cmds: Vec<Message> = got
        .into_iter()
        .filter_map(|o| match o {
            Out::Cmd(m) => Some(m),
            Out::Raw(_) => None,
        })
        .collect();
    assert_eq!(cmds.len(), 1, "one command in {out}");
    cmds.into_iter().next().unwrap()
}

fn msg(pairs: &[(&str, &str)], payload: &[u8]) -> Message {
    Message::new(pairs.iter().copied().collect(), payload)
}

// --- a program's commands --------------------------------------------------

#[test]
fn a_delta_reads_back_as_its_ops() {
    let ops = vec![
        text("a", "hi"),
        var("bar", "w", "40%"),
        attr("ok", "title", "Save"),
        unattr("ok", "disabled"),
        remove("old"),
        reveal_op("s3", "end"),
        reveal_op("s4", ""),
        morph("", "<p id=a>1</p>"),
        text("empty", ""),
    ];
    assert_eq!(delta_of(&command(delta("x", &ops))).unwrap(), ops);
}

#[test]
fn a_delta_a_host_refuses_names_the_op_at_fault() {
    let refused = |ops: &[Op]| delta_of(&command(delta("x", ops))).unwrap_err();
    let e = refused(&[]);
    assert_eq!((e.code.as_str(), e.op), ("EINVAL", None));
    let bad = |op: &str, target: &str, key: &str, value: Option<&str>| Op {
        op: op.into(),
        target: target.into(),
        key: key.into(),
        value: value.map(Into::into),
    };
    for (op, at) in [
        (bad("shout", "a", "", Some("x")), 1),
        (bad("text", "", "", Some("x")), 1),
        (bad("attr", "a", "", Some("x")), 1),
    ] {
        let e = refused(&[text("a", "ok"), op.clone()]);
        assert_eq!((e.code.as_str(), e.op), ("EINVAL", Some(at)), "{op:?}");
        assert_eq!(e.re, "delta");
        assert_eq!(e.surface.as_deref(), Some("x"));
    }
    // An op's field of another type: a body no builder writes.
    let body = [
        0x81, 0xa3, b'o', b'p', b's', 0x91, 0x82, 0xa2, b'o', b'p', 0xa4, b't', b'e', b'x', b't',
        0xa1, b't', 0x05,
    ];
    let e = delta_of(&msg(&[("a", "delta"), ("s", "x")], &body)).unwrap_err();
    assert_eq!(e.op, Some(0));
    // {"ops": [{"op": "reveal", "t": "a", "v": "middle"}]}: a reveal's
    // value the builder never writes.
    let mut body = vec![0x81, 0xa3, b'o', b'p', b's', 0x91, 0x83];
    for (k, v) in [("op", "reveal"), ("t", "a"), ("v", "middle")] {
        body.push(0xa0 | k.len() as u8);
        body.extend_from_slice(k.as_bytes());
        body.push(0xa0 | v.len() as u8);
        body.extend_from_slice(v.as_bytes());
    }
    let e = delta_of(&msg(&[("a", "delta"), ("s", "x")], &body)).unwrap_err();
    assert_eq!(e.op, Some(0));
    let e = delta_of(&msg(&[("a", "delta"), ("s", "x")], b"\xc0")).unwrap_err();
    assert_eq!(e.op, None, "a body that does not decode");
}

#[test]
fn a_value_an_op_does_not_take_is_dropped() {
    let op = Op {
        op: "remove".into(),
        target: "a".into(),
        key: String::new(),
        value: Some("ignored".into()),
    };
    let ops = delta_of(&command(delta("x", &[op]))).unwrap();
    assert_eq!(ops[0].value, None);
}

#[test]
fn a_menu_reads_back_as_its_tree() {
    let tree = MenuTree {
        label: "Files".into(),
        items: vec![
            MenuItem {
                key: "space".into(),
                label: "Leader".into(),
                items: vec![MenuItem {
                    key: "q".into(),
                    label: "Quit".into(),
                    name: "quit".into(),
                    ..MenuItem::default()
                }],
                ..MenuItem::default()
            },
            MenuItem {
                name: "sort".into(),
                enabled: false,
                ..MenuItem::default()
            },
        ],
    };
    assert_eq!(menu_of(&command(menu(Some(&tree)))).unwrap(), Some(tree));
    assert_eq!(menu_of(&command(menu(None))).unwrap(), None);
    assert_eq!(
        menu_show_of(&command(menu_show(["space", "f"]))).unwrap(),
        ["space", "f"]
    );
    assert!(
        menu_show_of(&command(menu_show::<&str>([])))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_menu_nested_too_deep_is_refused() {
    let mut item = MenuItem {
        key: "a".into(),
        ..MenuItem::default()
    };
    for _ in 0..MENU_DEPTH {
        item = MenuItem {
            key: "a".into(),
            items: vec![item],
            ..MenuItem::default()
        };
    }
    let tree = MenuTree {
        label: String::new(),
        items: vec![item],
    };
    assert!(menu_of(&command(menu(Some(&tree)))).is_err());
}

#[test]
fn touch_areas_read_back() {
    let areas = vec![
        TouchArea {
            c: 2,
            r: 3,
            w: 10,
            h: 1,
            drag: TouchDrag::X,
        },
        TouchArea {
            c: 0,
            r: 5,
            w: 1,
            h: 4,
            drag: TouchDrag::Xy,
        },
    ];
    assert_eq!(touch_of(&command(touch(&areas))).unwrap(), areas);
    assert!(touch_of(&command(touch(&[]))).unwrap().is_empty());
    let zero = TouchArea { w: 0, ..areas[0] };
    assert!(touch_of(&command(touch(&[zero]))).is_err());
}

#[test]
fn a_field_change_reads_back() {
    let changes = [
        FieldChange {
            rev: 2,
            from: Some(0),
            to: Some(2),
            text: Some("src/".into()),
            ..FieldChange::default()
        },
        FieldChange {
            rev: 0,
            value: Some(String::new()),
            sel: Some((3, 0)),
            ..FieldChange::default()
        },
        FieldChange {
            rev: 7,
            checked: Some(false),
            ..FieldChange::default()
        },
    ];
    for c in changes {
        assert_eq!(field_of(&command(set_field("find", "q", &c))).unwrap(), c);
    }
}

#[test]
fn a_field_change_that_needs_no_document_to_refuse_is_einval() {
    let field = |payload: &[u8]| msg(&[("a", "field"), ("s", "x"), ("t", "q")], payload);
    let bodies: [&[u8]; 6] = [
        b"",
        b"\x91\x00",                                               // [0]
        b"\x81\xa5value\xa1x",                                     // {"value": "x"}
        b"\x82\xa3rev\xa10\xa5value\xa1x",                         // {"rev": "0", "value": "x"}
        b"\x82\xa3rev\x00\xa3sel\x91\x01",                         // {"rev": 0, "sel": [1]}
        b"\x82\xa3rev\x00\xa3sel\x92\x00\xcb\x3f\xf0\0\0\0\0\0\0", // {"rev": 0, "sel": [0, 1.0]}
    ];
    for b in bodies {
        let e = field_of(&field(b)).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.re.as_str()),
            (code::EINVAL, "field"),
            "{b:?}"
        );
    }
    let text = |t: &str| Some(t.to_string());
    let changes = [
        FieldChange::default(),
        FieldChange {
            value: text("x"),
            text: text("y"),
            ..FieldChange::default()
        },
        FieldChange {
            value: text("x"),
            from: Some(0),
            to: Some(0),
            ..FieldChange::default()
        },
        FieldChange {
            from: Some(0),
            text: text("y"),
            ..FieldChange::default()
        },
        FieldChange {
            from: Some(0),
            to: Some(1),
            ..FieldChange::default()
        },
        FieldChange {
            from: Some(2),
            to: Some(1),
            text: text("y"),
            ..FieldChange::default()
        },
        FieldChange {
            sel: Some((-1, 0)),
            ..FieldChange::default()
        },
    ];
    for c in changes {
        assert!(
            field_of(&command(set_field("x", "q", &c))).is_err(),
            "{c:?}"
        );
    }
    let no_t = msg(
        &[("a", "field"), ("s", "x")],
        b"\x82\xa3rev\x00\xa5value\xa1x",
    );
    assert_eq!(field_of(&no_t).unwrap_err().code, code::EINVAL);
}

#[test]
fn a_field_fits_its_control_or_is_einval() {
    assert_eq!(control_kind("input", None), Some(ControlKind::Text));
    assert_eq!(
        control_kind("INPUT", Some("Email")),
        Some(ControlKind::Text)
    );
    assert_eq!(
        control_kind("input", Some("whatever")),
        Some(ControlKind::Text)
    );
    assert_eq!(control_kind("textarea", None), Some(ControlKind::Text));
    assert_eq!(
        control_kind("input", Some("radio")),
        Some(ControlKind::Check)
    );
    assert_eq!(control_kind("select", None), Some(ControlKind::Select));
    assert_eq!(
        control_kind("input", Some("range")),
        Some(ControlKind::Other)
    );
    assert_eq!(control_kind("input", Some("submit")), None);
    assert_eq!(control_kind("p", None), None);
    let m = msg(&[("a", "field"), ("s", "x"), ("t", "q")], b"");
    let value = FieldChange {
        value: Some("x".into()),
        ..FieldChange::default()
    };
    let sel = FieldChange {
        sel: Some((0, 0)),
        ..FieldChange::default()
    };
    let checked = FieldChange {
        checked: Some(true),
        ..FieldChange::default()
    };
    let text = FieldChange {
        text: Some("a".into()),
        ..FieldChange::default()
    };
    use ControlKind::*;
    for (c, kind, fits) in [
        (&value, Text, true),
        (&sel, Text, true),
        (&text, Text, true),
        (&checked, Text, false),
        (&checked, Check, true),
        (&value, Check, false),
        (&sel, Check, false),
        (&value, Select, true),
        (&text, Select, false),
        (&value, Other, true),
        (&sel, Other, false),
    ] {
        assert_eq!(field_fits(&m, c, kind).is_ok(), fits, "{c:?} {kind:?}");
    }
}

#[test]
fn a_placement_reads_back() {
    for p in [
        Placement::new(40, 0),
        Placement {
            cols: 40,
            rows: 10,
            window: Some(Window {
                x: 0,
                y: 2,
                w: 40,
                h: 5,
            }),
            z: -3,
            press: Some(true),
            fit: Some(false),
            hover: None,
            keep_cursor: true,
        },
    ] {
        assert_eq!(placement_of(&command(place("s", &p))).unwrap(), p);
    }
    // Defaults fill the window in; with auto rows, h is the host's to find.
    let m = msg(
        &[
            ("a", "place"),
            ("s", "s"),
            ("c", "40"),
            ("r", "10"),
            ("y", "2"),
        ],
        b"",
    );
    assert_eq!(
        placement_of(&m).unwrap().window,
        Some(Window {
            x: 0,
            y: 2,
            w: 40,
            h: 8
        })
    );
    let m = msg(&[("a", "place"), ("s", "s"), ("c", "40"), ("x", "4")], b"");
    assert_eq!(
        placement_of(&m).unwrap().window,
        Some(Window {
            x: 4,
            y: 0,
            w: 36,
            h: 0
        })
    );
    for bad in [
        &[("a", "place"), ("s", "s")][..],
        &[("a", "place"), ("s", "s"), ("c", "0")],
        &[("a", "place"), ("s", "s"), ("c", "1001")],
        &[("a", "place"), ("s", "s"), ("c", "4"), ("r", "x")],
        &[("a", "place"), ("s", "s"), ("c", "4"), ("w", "5")],
        &[
            ("a", "place"),
            ("s", "s"),
            ("c", "4"),
            ("r", "2"),
            ("y", "2"),
        ],
        &[("a", "place"), ("s", "s"), ("c", "4"), ("z", "1001")],
    ] {
        let e = placement_of(&msg(bad, b"")).unwrap_err();
        assert_eq!(e.code, "EINVAL", "{bad:?}");
    }
    assert_eq!(cursor_below(2), "\x1bD\x1bD\r");
}

#[test]
fn the_quiet_level_is_the_wires() {
    let q = |pairs: &[(&str, &str)]| quiet_of(&msg(pairs, b""));
    assert_eq!(q(&[("a", "doc")]), Quiet::ReplyOnError);
    assert_eq!(q(&[("a", "doc"), ("n", "3")]), Quiet::ReplyAlways);
    assert_eq!(q(&[("a", "q")]), Quiet::ReplyAlways);
    assert_eq!(q(&[("a", "doc"), ("n", "3"), ("q", "2")]), Quiet::NoReply);
    assert_eq!(q(&[("a", "doc"), ("q", "7")]), Quiet::ReplyOnError);
    assert!(speaks(&command(query(1))));
    assert!(speaks(&msg(&[("a", "q"), ("v", "0.4,0.5")], b"")));
    assert!(!speaks(&msg(&[("a", "q"), ("v", "0.4")], b"")));
}

// --- a host's replies and events ---------------------------------------------

fn reply_of(s: String) -> Reply {
    command(s).reply().expect("a reply")
}

fn event_of(s: String) -> Event {
    command(s).event().expect("an event")
}

#[test]
fn replies_read_back() {
    let cmd = command(place("card", &Placement::new(40, 0)).n(7));
    let r = reply_of(reply_ok(&cmd, &[("c", "40"), ("r", "3")], b""));
    assert_eq!(
        (
            r.ok,
            r.re.as_str(),
            r.n,
            r.surface.as_deref(),
            r.cols,
            r.rows
        ),
        (true, "place", Some(7), Some("card"), Some(40), Some(3))
    );

    let cmd = command(delta("card", &[text("a", "1"), text("b", "2")]));
    let err = Error {
        code: code::ENOTARGET.into(),
        detail: "b".into(),
        op: Some(1),
        ..Error::default()
    };
    let r = reply_of(reply_err(&cmd, &err));
    assert_eq!(
        (r.ok, r.code.as_deref(), r.detail.as_deref(), r.op),
        (false, Some("ENOTARGET"), Some("b"), Some(1))
    );
}

fn full_caps() -> Caps {
    Caps {
        v: Some(VERSION.into()),
        cell_css: Some(CellCss { w: 8.0, h: 16.0 }),
        scale: Some(2.0),
        scheme: Some("light".into()),
        modifier: Some("super".into()),
        features: Some(vec!["hover".into(), "menu".into()]),
        limits: Some(BTreeMap::from([
            ("surfaces".into(), 64),
            ("resources".into(), i64::MAX),
        ])),
        net: Some(BTreeMap::from([(
            "img-src".into(),
            vec!["https://example.com".into()],
        )])),
        edit: Some(vec![("copy".into(), vec!["super+c".into()])]),
        host: Some("test".into()),
        version: Some("1.2.3".into()),
    }
}

#[test]
fn the_capabilities_read_back() {
    let caps = full_caps();
    let r = reply_of(reply_query(&command(query(1)), &caps));
    let mut want = caps.clone();
    want.limits
        .as_mut()
        .unwrap()
        .insert("resources".into(), (1 << 53) - 1);
    assert_eq!(r.caps, Some(want), "an int is held within 2^53 - 1");
    assert_eq!(r.n, Some(1));

    let old = msg(&[("a", "q"), ("n", "1"), ("v", "0.3")], b"");
    let r = reply_of(reply_query(&old, &caps));
    assert_eq!(
        (r.ok, r.code.as_deref(), r.detail.as_deref()),
        (false, Some("EVERSION"), Some(VERSION))
    );
    assert_eq!(
        hotty::host::caps_body(&Caps::default()),
        [0x80],
        "none: an empty map"
    );
    let r = reply_of(reply_caps(&command(query(1)), &Caps::default()));
    assert_eq!(r.caps, Some(Caps::default()));
}

fn ev(kind: &str, surface: Option<&str>, target: &str) -> Event {
    Event {
        kind: kind.into(),
        surface: surface.map(Into::into),
        target: target.into(),
        ..Event::default()
    }
}

#[test]
fn events_read_back() {
    let cells = Area {
        c: -1,
        r: 2,
        w: 3,
        h: 4,
    };
    let area = Some(cells);
    let events = vec![
        Event {
            area,
            mods: MOD_SHIFT | MOD_CTRL,
            value: Some("v".into()),
            count: Some(2),
            link: Some(Link {
                href: "#a".into(),
                url: Some("https://x/#a".into()),
            }),
            ..ev("click", Some("s"), "ok")
        },
        Event {
            count: Some(1),
            ..ev("click", Some("s"), "ok")
        },
        Event {
            area,
            ..ev("press", Some("s"), "")
        },
        Event {
            checked: Some(true),
            value: Some("on".into()),
            ..ev("change", Some("s"), "box")
        },
        Event {
            value: Some("typed".into()),
            ..ev("input", Some("s"), "f")
        },
        Event {
            fields: Some(vec![("a".into(), "1".into()), ("a".into(), "2".into())]),
            ..ev("submit", Some("s"), "form")
        },
        Event {
            fields: Some(vec![]),
            ..ev("submit", Some("s"), "form")
        },
        Event {
            fit_rows: Some(7),
            ..ev("fit", Some("s"), "")
        },
        Event {
            drag: Some(Drag {
                c: -2,
                r: 300,
                x: Some(0),
                y: None,
                tx: Some(4),
                ty: None,
            }),
            mods: MOD_ALT,
            ..ev("drag", Some("s"), "knob")
        },
        Event {
            hover: Some(Hover {
                c: Some(3),
                r: Some(1),
                out: false,
            }),
            ..ev("hover", Some("s"), "row")
        },
        Event {
            hover: Some(Hover {
                c: None,
                r: None,
                out: true,
            }),
            ..ev("hover", Some("s"), "")
        },
        Event {
            view: Some(View {
                areas: BTreeMap::from([("a".into(), cells)]),
                at_end: true,
            }),
            ..ev("view", Some("s"), "")
        },
        Event {
            command: Some(Command {
                name: "paste".into(),
                key: "ctrl+v".into(),
                text: "hi".into(),
                has_text: true,
            }),
            ..ev("command", None, "")
        },
        Event {
            command: Some(Command {
                name: "copy".into(),
                ..Command::default()
            }),
            ..ev("command", None, "")
        },
        Event {
            changed: Some(Caps {
                scheme: Some("dark".into()),
                features: Some(vec![]),
                ..Caps::default()
            }),
            ..ev("caps", None, "")
        },
        ev("focus", Some("s"), "name"),
        ev("blur", Some("s"), ""),
    ];
    for want in events {
        let s = encode_event(&want);
        let mut got = event_of(s.clone());
        got.message = Message::default();
        assert_eq!(got, want, "{s:?}");
    }
}

#[test]
fn a_relay_passes_a_detail_on_as_it_came() {
    let mut e = event_of(encode_event(&Event {
        value: Some("x".into()),
        ..ev("input", Some("inner"), "f")
    }));
    e.surface = Some("outer-inner".into());
    let again = event_of(encode_event(&e));
    assert_eq!(again.surface.as_deref(), Some("outer-inner"));
    assert_eq!(again.value.as_deref(), Some("x"));
    assert_eq!(again.message.payload, e.message.payload);
}

// --- a program's output, split ---------------------------------------------

// What a splitter hands on, as owned pieces, adjacent bytes joined.
#[derive(Debug, PartialEq)]
enum Piece {
    Bytes(Vec<u8>),
    Seq(Seq, Vec<u8>),
    Command(Message),
    Invalid(&'static str),
}

fn split(pieces: &[&[u8]]) -> Vec<Piece> {
    let mut s = Splitter::new();
    let mut out: Vec<Piece> = Vec::new();
    for p in pieces {
        s.feed(p, &mut |o| match o {
            Output::Bytes(b) => match out.last_mut() {
                Some(Piece::Bytes(last)) => last.extend_from_slice(b),
                _ => out.push(Piece::Bytes(b.to_vec())),
            },
            Output::Seq(q, b) => out.push(Piece::Seq(q, b.to_vec())),
            Output::Command(m) => out.push(Piece::Command(m)),
            Output::Invalid(w) => out.push(Piece::Invalid(w)),
        });
    }
    out
}

fn split_every_way(stream: &[u8]) -> Vec<Piece> {
    let whole = split(&[stream]);
    for at in 0..=stream.len() {
        assert_eq!(split(&[&stream[..at], &stream[at..]]), whole, "cut at {at}");
    }
    let bytes: Vec<&[u8]> = stream.chunks(1).collect();
    assert_eq!(split(&bytes), whole, "byte by byte");
    whole
}

#[test]
fn output_splits_into_bytes_sequences_and_commands() {
    let doc_cmd = doc("card", "<p>hi</p>").to_string();
    let stream = [
        b"a\x1b[31m".as_slice(),
        b"\x1b[?1049h",
        doc_cmd.as_bytes(),
        b"\x1b]0;title\x07",
        b"\x1b[2J",
        b"\x1bc",
        b"z",
    ]
    .concat();
    let got = split_every_way(&stream);
    assert_eq!(
        got,
        vec![
            Piece::Bytes(b"a\x1b[31m".to_vec()),
            Piece::Seq(
                Seq::PrivateMode {
                    set: true,
                    modes: vec![1049]
                },
                b"\x1b[?1049h".to_vec()
            ),
            Piece::Command(Message::new(
                Control::from([("a", "doc"), ("s", "card")]),
                b"<p>hi</p>".to_vec()
            )),
            Piece::Bytes(b"\x1b]0;title\x07".to_vec()),
            Piece::Seq(Seq::EraseDisplay(2), b"\x1b[2J".to_vec()),
            Piece::Seq(Seq::Reset, b"\x1bc".to_vec()),
            Piece::Bytes(b"z".to_vec()),
        ]
    );
}

#[test]
fn a_chunked_command_comes_out_whole() {
    let data: Vec<u8> = (0..20_000u32).map(|i| (i * 7919 % 251) as u8).collect();
    let cmd = res("big", "application/octet-stream", &data).to_string();
    let stream = [b"x".as_slice(), cmd.as_bytes(), b"y"].concat();
    let got = split(&[&stream]);
    assert_eq!(got.len(), 3);
    let Piece::Command(m) = &got[1] else {
        panic!("{got:?}")
    };
    assert_eq!(m.payload, data);
    let bytes: Vec<&[u8]> = stream.chunks(997).collect();
    assert_eq!(split(&bytes), got);
}

#[test]
fn malformed_commands_are_dropped_and_said_why() {
    let got = split_every_way(b"\x1b]7279;a=ok\x1b[A");
    assert_eq!(
        got,
        vec![
            Piece::Invalid("hotty command cut short by ESC"),
            Piece::Bytes(b"\x1b[A".to_vec()),
        ]
    );
    let got = split(&[b"\x1b]7279;a b\x1b\\"]);
    assert!(matches!(got.as_slice(), [Piece::Invalid(_)]), "{got:?}");
    // A chunk cut short by another command: both reported, the second runs.
    let got = split(&[b"\x1b]7279;a=res:id=x:m=1;AAAA\x1b\\\x1b]7279;a=del\x1b\\"]);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(matches!(got[0], Piece::Invalid(_)));
    assert!(matches!(&got[1], Piece::Command(m) if m.action() == "del"));
}

#[test]
fn a_reset_inside_another_string_is_no_reset() {
    let got = split_every_way(b"\x1bPq\x1bc\x1b\\");
    assert_eq!(got, vec![Piece::Bytes(b"\x1bPq\x1bc\x1b\\".to_vec())]);
}

// --- keys, for a host ----------------------------------------------------------

#[test]
fn key_parts_split_and_join() {
    let k = KeyParts::split("ctrl+shift+arrow_left");
    assert_eq!(k.kind(), Kind::Physical);
    assert!(k.shift());
    assert_eq!(k.name(), "ctrl+shift+arrow_left");
    assert_eq!(
        KeyParts::new(false, true, false, true, "a").name(),
        "ctrl+shift+a"
    );
    assert_eq!(char_key(SHIFT, "A"), "shift+a");
    assert_eq!(char_key(SHIFT, "!"), "!");
    assert_eq!(char_key(0, " "), "space");
}

#[test]
fn a_press_is_asked_by_the_name_the_keymap_binds() {
    let m = parse_keymap(&Platform::default(), "ctrl+key_w=program");
    let names = vec!["ctrl+key_w".to_string(), "ctrl+w".to_string()];
    assert_eq!(m.press(&names), Some("ctrl+key_w"));
    let p = Platform::new(Mod::Super);
    assert_eq!(p.edit_action("super+c").as_deref(), Some("copy"));
    assert_eq!(
        p.edit_press(&["super+key_v".into(), "super+v".into()])
            .as_deref(),
        Some("paste")
    );
    assert_eq!(p.edit_action("ctrl+c"), None);
}

// --- what hotty-blitz's hotty-wire checked of its scanner, kept -------------

/// A tiny deterministic PRNG, so the property tests need no dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Everything a splitter hands on, flattened: the bytes passed on (a
/// sequence's included), the sequences, the commands and the drops.
#[derive(Default)]
struct Collected {
    bytes: Vec<u8>,
    seqs: Vec<Seq>,
    commands: Vec<Message>,
    invalid: Vec<&'static str>,
}

fn collect(s: &mut Splitter, pieces: impl Iterator<Item = Vec<u8>>) -> Collected {
    let mut out = Collected::default();
    for p in pieces {
        s.feed(&p, &mut |o| match o {
            Output::Bytes(b) => out.bytes.extend_from_slice(b),
            Output::Seq(q, b) => {
                out.bytes.extend_from_slice(b);
                out.seqs.push(q);
            }
            Output::Command(m) => out.commands.push(m),
            Output::Invalid(w) => out.invalid.push(w),
        });
    }
    out
}

// Fed in random pieces of 1 to 40 bytes.
fn split_randomly(input: &[u8], rng: &mut Lcg) -> Collected {
    let mut pieces = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let n = 1 + rng.below(40.min(input.len() - i));
        pieces.push(input[i..i + n].to_vec());
        i += n;
    }
    collect(&mut Splitter::new(), pieces.into_iter())
}

/// Random terminal output that holds no hotty command.
fn noise(rng: &mut Lcg, len: usize) -> Vec<u8> {
    let pieces: [&[u8]; 14] = [
        b"hello ",
        "héllo wörld Ü ".as_bytes(),
        b"\x1b[1;31m",
        b"\x1b[0m",
        b"\x1b]0;title\x07",
        b"\x1b]8;;https://x.y\x1b\\",
        b"\x1b]72791;not ours\x07",
        b"\x1b]72;t=a:i=1\x1b\\",
        b"\x1bP1$r0m\x1b\\",
        b"\x1b_Ga=q,i=1;AAAA\x1b\\",
        b"\x1b[?25l",
        b"\x1b[H\x1b[2J",
        b"\r\n",
        b"\x1b\x1b[A",
    ];
    let mut out = Vec::new();
    while out.len() < len {
        if rng.below(4) == 0 {
            out.push((rng.below(95) + 32) as u8);
        } else {
            out.extend_from_slice(pieces[rng.below(pieces.len())]);
        }
    }
    out
}

#[test]
fn what_is_not_hottys_passes_byte_exact_however_it_is_split() {
    let mut rng = Lcg(7);
    for round in 0..300 {
        let input = noise(&mut rng, 50 + round * 3);
        let got = split_randomly(&input, &mut rng);
        assert_eq!(got.bytes, input, "round {round}");
        assert!(got.commands.is_empty());
        assert!(got.invalid.is_empty(), "{:?}", got.invalid);
    }
}

#[test]
fn commands_come_out_whole_among_noise_however_it_is_split() {
    let mut rng = Lcg(42);
    let big: String = (0..5000)
        .map(|i| format!("<div id=r{i}>row {i}</div>"))
        .collect();
    let random_big: Vec<u8> = (0..20000).map(|_| rng.next() as u8).collect();
    let commands = [
        msg(&[("a", "q"), ("n", "1")], b""),
        msg(
            &[("a", "doc"), ("s", "main")],
            "<p>héllo</p>\n<pre>a\n\tb</pre>".as_bytes(),
        ),
        msg(&[("a", "doc"), ("s", "big")], big.as_bytes()),
        msg(
            &[
                ("a", "res"),
                ("id", "blob"),
                ("type", "application/octet-stream"),
            ],
            &random_big,
        ),
    ];
    for round in 0..40 {
        let (mut input, mut passed) = (Vec::new(), Vec::new());
        for c in &commands {
            let n = noise(&mut rng, 30);
            input.extend_from_slice(&n);
            passed.extend_from_slice(&n);
            input.extend_from_slice(encode(&c.control, &c.payload).as_bytes());
        }
        let got = split_randomly(&input, &mut rng);
        assert!(got.invalid.is_empty(), "round {round}: {:?}", got.invalid);
        assert_eq!(got.bytes, passed, "round {round}");
        assert_eq!(got.commands, commands, "round {round}");
    }
}

#[test]
fn bel_ends_a_command_and_the_sequences_a_host_acts_on_are_said() {
    let input = b"\x1b[?1049h\x1b[?2026;1000l\x1b]7279;a=q:n=9\x07\x1b[2J\x1bcdone";
    let got = split_randomly(input, &mut Lcg(1));
    assert_eq!(got.commands, [msg(&[("a", "q"), ("n", "9")], b"")]);
    assert_eq!(
        got.seqs,
        [
            Seq::PrivateMode {
                set: true,
                modes: vec![1049]
            },
            Seq::PrivateMode {
                set: false,
                modes: vec![2026, 1000]
            },
            Seq::EraseDisplay(2),
            Seq::Reset,
        ]
    );
    assert_eq!(got.bytes, b"\x1b[?1049h\x1b[?2026;1000l\x1b[2J\x1bcdone");
}

fn with_max(input: &[u8], max: usize) -> Collected {
    collect(
        &mut Splitter::with_max(max),
        std::iter::once(input.to_vec()),
    )
}

#[test]
fn the_limit_is_on_decoded_bytes() {
    for (n, ok) in [(4096, true), (4097, false)] {
        let cmd = res("x", "application/octet-stream", &vec![0u8; n]);
        let got = with_max(encode_plain(&cmd.control(), cmd.payload()).as_bytes(), 4096);
        assert_eq!(got.commands.len(), usize::from(ok), "{n} bytes");
        assert_eq!(got.invalid.len(), usize::from(!ok), "{n} bytes");
    }
}

#[cfg(feature = "zlib")]
#[test]
fn zlib_past_the_limit_is_refused_not_cut_short() {
    let cmd = res("x", "application/octet-stream", &[0; 4096]);
    let packed = encode(&cmd.control(), cmd.payload());
    assert!(packed.contains("o=z"));
    assert_eq!(
        with_max(packed.as_bytes(), 4096).commands[0].payload.len(),
        4096
    );
    let got = with_max(packed.as_bytes(), 4095);
    assert!(got.commands.is_empty());
    assert_eq!(got.invalid.len(), 1);
}

#[test]
fn a_chunked_command_past_the_limit_is_dropped_whole_and_said_once() {
    let chunk = "A".repeat(400);
    let mut input = format!("\x1b]7279;a=doc:s=x:m=1;{chunk}\x1b\\");
    for _ in 0..5 {
        input += &format!("\x1b]7279;m=1;{chunk}\x1b\\");
    }
    input += &format!("\x1b]7279;m=0;{chunk}\x1b\\\x1b]7279;a=del:s=x\x1b\\");
    let got = with_max(input.as_bytes(), 1024);
    assert_eq!(got.invalid.len(), 1, "{:?}", got.invalid);
    assert_eq!(got.commands, [msg(&[("a", "del"), ("s", "x")], b"")]);
    assert!(got.bytes.is_empty());
}

#[test]
fn the_chunks_after_one_too_large_to_hold_are_dropped_too() {
    let mut input = b"\x1b]7279;a=doc:s=x:m=1;".to_vec();
    input.extend(std::iter::repeat_n(b'A', 8000));
    input.extend_from_slice(b"\x1b\\\x1b]7279;m=1;AAAA\x1b\\\x1b]7279;m=0;AAAA\x1b\\");
    input.extend_from_slice(b"\x1b]7279;a=del:s=x\x1b\\");
    let got = with_max(&input, 1024);
    assert_eq!(got.invalid, ["hotty command too large"]);
    assert_eq!(got.commands, [msg(&[("a", "del"), ("s", "x")], b"")]);
}
