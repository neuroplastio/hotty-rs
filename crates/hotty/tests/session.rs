//! The Session: layout, relayout, lost documents, limits and pings (SDK.md §4.3).

use hotty::link::Link;
use hotty::session::{Mode, Rect, Session, SessionMessage, Want};
use hotty::{Control, encode_plain};

const CAPS_05: &[u8] = &[0x81, 0xa1, b'v', 0xa3, b'0', b'.', b'5'];
// {"v": "0.5", "limits": {"surfaces": 2}}
const CAPS_LIMIT_2: &[u8] = &[
    0x82, 0xa1, b'v', 0xa3, b'0', b'.', b'5', 0xa6, b'l', b'i', b'm', b'i', b't', b's', 0x81, 0xa8,
    b's', b'u', b'r', b'f', b'a', b'c', b'e', b's', 0x02,
];

fn reply(pairs: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    encode_plain(&pairs.iter().copied().collect::<Control>(), body).into_bytes()
}

fn native_with(caps: &[u8]) -> (Session, Link) {
    let mut link = Link::new("p");
    let mut s = Session::new();
    s.detect(&mut link, 0);
    link.take_output();
    let mut b = reply(&[("a", "ok"), ("n", "1"), ("re", "q")], caps);
    b.extend_from_slice(b"\x1b[?62;22c");
    link.bytes(&b, 5);
    assert!(matches!(
        s.poll(&mut link, 5),
        Some(SessionMessage::Ready {
            mode: Mode::Native,
            ..
        })
    ));
    assert_eq!(s.poll(&mut link, 5), None);
    (s, link)
}

fn native() -> (Session, Link) {
    native_with(CAPS_05)
}

fn want(name: &str, x: i32, y: i32) -> Want<'static> {
    let n = name.to_string();
    Want::new(name, Rect::new(x, y, 10, 3), move || format!("<p>{n}</p>"))
}

fn actions(out: &str) -> Vec<String> {
    out.split("\x1b]7279;")
        .skip(1)
        .map(|s| {
            let ctl = s.split([';', '\x1b']).next().unwrap();
            let a = ctl.split(':').next().unwrap().trim_start_matches("a=");
            let s = ctl
                .split(':')
                .find_map(|kv| kv.strip_prefix("s="))
                .unwrap_or("");
            format!("{a} {s}")
        })
        .collect()
}

#[test]
fn a_new_surface_gets_its_document_and_a_placement_once() {
    let (mut s, mut link) = native();
    s.layout([want("a", 2, 1)]);
    let out = s.flush(&mut link, 10);
    assert_eq!(actions(&out), ["doc a", "place a"]);
    assert!(
        out.contains("\x1b7\x1b[2;3H"),
        "placed at the cell: {out:?}"
    );
    s.layout([want("a", 2, 1)]);
    assert_eq!(s.flush(&mut link, 11), "", "nothing changed");
    s.layout([want("a", 4, 1)]);
    assert_eq!(actions(&s.flush(&mut link, 12)), ["place a"]);
    s.layout([]);
    assert_eq!(actions(&s.flush(&mut link, 13)), ["del a"]);
    assert!(!s.has("a"));
}

#[test]
fn a_kept_surface_is_hidden_and_shown_again_without_its_document() {
    let (mut s, mut link) = native();
    let mut w = want("a", 0, 0);
    w.keep = true;
    s.layout([w]);
    s.flush(&mut link, 10);
    s.layout([]);
    assert_eq!(actions(&s.flush(&mut link, 11)), ["hide a"]);
    let mut w = want("a", 0, 0);
    w.keep = true;
    s.layout([w]);
    assert_eq!(actions(&s.flush(&mut link, 12)), ["place a"]);
}

#[test]
fn flags_go_out_only_when_they_change() {
    let (mut s, mut link) = native();
    let mut w = want("a", 0, 0);
    w.press = true;
    s.layout([w]);
    assert!(s.flush(&mut link, 10).contains(":p=1"));
    let mut w = want("a", 1, 0);
    w.press = true;
    s.layout([w]);
    let out = s.flush(&mut link, 11);
    assert!(!out.contains(":p="), "{out:?}");
    s.layout([want("a", 1, 0)]);
    assert!(s.flush(&mut link, 12).contains(":p=0"));
}

#[test]
fn a_clip_places_the_window_inside_it() {
    let (mut s, mut link) = native();
    let mut w = Want::new("a", Rect::new(0, -2, 10, 5), || "x".into());
    w.clip = Some(Rect::new(0, 0, 80, 24));
    s.layout([w]);
    let out = s.flush(&mut link, 10);
    assert!(out.contains("c=10:r=5:x=0:y=2:w=10:h=3"), "{out:?}");
    let mut w = Want::new("a", Rect::new(0, -9, 10, 5), || "x".into());
    w.clip = Some(Rect::new(0, 0, 80, 24));
    s.layout([w]);
    assert_eq!(actions(&s.flush(&mut link, 11)), ["del a"], "out of view");
}

#[test]
fn an_erase_asks_for_a_relayout_and_places_again() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    s.watch(b"\x1b[H\x1b[2");
    assert_eq!(s.poll(&mut link, 11), None);
    s.watch(b"J");
    assert_eq!(s.poll(&mut link, 12), Some(SessionMessage::Relayout));
    s.layout([want("a", 0, 0)]);
    assert_eq!(actions(&s.flush(&mut link, 13)), ["place a"]);
    assert_eq!(s.counts().relayouts, 1);
}

#[test]
fn a_layout_after_the_erase_makes_the_relayout_moot() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    s.watch(b"\x1b[2J");
    s.layout([want("a", 0, 0)]);
    assert_eq!(actions(&s.flush(&mut link, 11)), ["place a"]);
    assert_eq!(s.poll(&mut link, 12), None);
}

#[test]
fn a_full_reset_sends_the_documents_again() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    s.watch(b"\x1bc");
    assert_eq!(s.poll(&mut link, 11), Some(SessionMessage::Relayout));
    s.layout([want("a", 0, 0)]);
    assert_eq!(actions(&s.flush(&mut link, 12)), ["doc a", "place a"]);
}

#[test]
fn a_lost_document_is_sent_again() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    link.bytes(
        &reply(
            &[("a", "err"), ("re", "place"), ("s", "a")],
            &[
                0x81, 0xa4, b'c', b'o', b'd', b'e', 0xa6, b'E', b'N', b'O', b'E', b'N', b'T',
            ],
        ),
        11,
    );
    assert_eq!(s.poll(&mut link, 11), Some(SessionMessage::Relayout));
    s.layout([want("a", 0, 0)]);
    assert_eq!(actions(&s.flush(&mut link, 12)), ["doc a", "place a"]);
}

#[test]
fn the_hosts_limit_holds_surfaces_back() {
    let (mut s, mut link) = native_with(CAPS_LIMIT_2);
    s.layout([want("a", 0, 0), want("b", 0, 3), want("c", 0, 6)]);
    assert_eq!(
        actions(&s.flush(&mut link, 10)),
        ["doc a", "place a", "doc b", "place b"]
    );
    s.layout([want("c", 0, 6)]);
    let out = actions(&s.flush(&mut link, 11));
    assert!(out.contains(&"doc c".to_string()), "{out:?}");
    assert!(out.contains(&"del a".to_string()) && out.contains(&"del b".to_string()));
}

#[test]
fn events_and_acks_reach_the_program_and_keys_pass() {
    let (mut s, mut link) = native();
    link.bytes(b"q", 10);
    link.bytes(
        &reply(&[("a", "ev"), ("s", "a"), ("e", "click"), ("t", "ok")], b""),
        11,
    );
    link.bytes(&reply(&[("a", "ok"), ("n", "7"), ("re", "focus")], b""), 12);
    assert_eq!(
        s.poll(&mut link, 12),
        Some(SessionMessage::Input(b"q".to_vec()))
    );
    let Some(SessionMessage::Event(ev)) = s.poll(&mut link, 12) else {
        panic!("no event")
    };
    assert_eq!((ev.kind.as_str(), ev.target.as_str()), ("click", "ok"));
    assert!(matches!(s.poll(&mut link, 12), Some(SessionMessage::Ack(r)) if r.n == Some(7)));
}

#[test]
fn a_ping_goes_with_the_next_flush_and_pongs() {
    let (mut s, mut link) = native();
    s.ping(100);
    assert!(s.flush(&mut link, 110).ends_with("\x1b[c"));
    link.bytes(b"\x1b[?62;22c", 125);
    assert_eq!(
        s.poll(&mut link, 125),
        Some(SessionMessage::Pong { took: 15 })
    );
}

#[test]
fn a_ping_with_no_frame_goes_on_its_own() {
    let (mut s, mut link) = native();
    s.ping(100);
    assert_eq!(s.deadline(), Some(200));
    assert_eq!(s.poll(&mut link, 200), None);
    assert_eq!(link.take_output(), b"\x1b[c");
}

#[test]
fn text_mode_sends_nothing() {
    let mut link = Link::new("p");
    let mut s = Session::new();
    s.detect(&mut link, 0);
    link.take_output();
    link.tick(1500);
    assert_eq!(
        s.poll(&mut link, 1500),
        Some(SessionMessage::Ready {
            mode: Mode::Text,
            caps: None
        })
    );
    s.layout([want("a", 0, 0)]);
    s.send("x");
    assert_eq!(s.flush(&mut link, 1501), "");
}

#[test]
fn close_deletes_what_it_sent() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0), want("b", 0, 3)]);
    s.flush(&mut link, 10);
    assert_eq!(actions(&s.close(&mut link, 11)), ["del a", "del b"]);
    s.layout([want("a", 0, 0)]);
    assert_eq!(s.flush(&mut link, 12), "");
}
