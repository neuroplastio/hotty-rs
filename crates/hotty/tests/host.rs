//! The host's side: a program's commands read as a host reads them, a
//! host's replies and events written, and a program's output split. Each
//! round trip goes through the other side, which the conformance vectors
//! check.

mod wire;

use std::collections::BTreeMap;

use hotty::host::{
    Output, Seq, Splitter, cursor_below, delta_of, encode_event, menu_of, menu_show_of,
    placement_of, quiet_of, reply_caps, reply_err, reply_ok, reply_query, speaks, touch_of,
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
    assert!(speaks(&msg(&[("a", "q"), ("v", "0.3,0.4")], b"")));
    assert!(!speaks(&msg(&[("a", "q"), ("v", "0.3")], b"")));
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
