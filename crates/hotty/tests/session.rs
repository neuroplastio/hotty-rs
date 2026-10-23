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

// The number and surface of the numbered document in what a flush returned:
// the one that marks a loss.
fn mark(out: &str) -> Option<(String, String)> {
    out.split("\x1b]7279;").skip(1).find_map(|m| {
        let ctl: Vec<&str> = m.split([';', '\x1b']).next()?.split(':').collect();
        if ctl.first() != Some(&"a=doc") {
            return None;
        }
        let n = ctl.iter().find_map(|kv| kv.strip_prefix("n="))?;
        let s = ctl.iter().find_map(|kv| kv.strip_prefix("s="))?;
        Some((n.to_string(), s.to_string()))
    })
}

// The host's answer to the document that marks a loss.
fn marked((n, s): &(String, String)) -> Vec<u8> {
    reply(&[("a", "ok"), ("n", n), ("s", s), ("re", "doc")], b"")
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
    // The host keeps its documents through an erase: nothing was lost.
    assert_eq!(
        s.poll(&mut link, 12),
        Some(SessionMessage::Relayout { lost: false })
    );
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

// The documents a full reset took go again, the first numbered as after an
// ENOENT's loss; its answer is the Session's.
#[test]
fn a_full_reset_sends_the_documents_again() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    s.watch(b"\x1bc");
    assert_eq!(
        s.poll(&mut link, 11),
        Some(SessionMessage::Relayout { lost: true })
    );
    s.layout([want("a", 0, 0)]);
    let out = s.flush(&mut link, 12);
    assert_eq!(actions(&out), ["doc a", "place a"]);
    let m = mark(&out).expect("a's document numbered");
    link.bytes(&marked(&m), 13);
    assert_eq!(s.poll(&mut link, 13), None, "the mark's answer");
}

// A full reset ends the session that stored the resources (SPEC §2), so
// after documents it is a loss, and takes the place of a plain relayout
// still waiting. Before any, it says nothing.
#[test]
fn a_full_reset_is_a_loss() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    s.watch(b"\x1b[2J");
    s.watch(b"\x1bc");
    assert_eq!(
        s.poll(&mut link, 11),
        Some(SessionMessage::Relayout { lost: true })
    );
    assert_eq!(s.poll(&mut link, 11), None);
    s.watch(b"\x1bc");
    assert_eq!(s.poll(&mut link, 12), None);
}

// The host's ENOENT for a placement of a surface it does not have.
fn gone(name: &str) -> Vec<u8> {
    reply(
        &[("a", "err"), ("re", "place"), ("s", name)],
        &[
            0x81, 0xa4, b'c', b'o', b'd', b'e', 0xa6, b'E', b'N', b'O', b'E', b'N', b'T',
        ],
    )
}

#[test]
fn a_lost_document_is_sent_again() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    link.bytes(&gone("a"), 11);
    assert_eq!(
        s.poll(&mut link, 11),
        Some(SessionMessage::Relayout { lost: true })
    );
    s.layout([want("a", 0, 0)]);
    assert_eq!(actions(&s.flush(&mut link, 12)), ["doc a", "place a"]);
}

// A hidden surface brought back after a loss goes with its document, rather
// than finding it gone and reporting the loss again.
#[test]
fn a_hidden_surface_goes_with_its_document_after_a_loss() {
    let (mut s, mut link) = native();
    let b = || {
        let mut w = want("b", 0, 3);
        w.keep = true;
        w
    };
    s.layout([want("a", 0, 0), b()]);
    s.flush(&mut link, 10);
    s.layout([want("a", 0, 0)]);
    assert_eq!(actions(&s.flush(&mut link, 11)), ["hide b"]);
    link.bytes(&gone("a"), 12);
    assert_eq!(
        s.poll(&mut link, 12),
        Some(SessionMessage::Relayout { lost: true })
    );
    assert!(!s.has("b"), "b's document is taken as kept after the loss");
    s.layout([want("a", 0, 0), b()]);
    assert_eq!(
        actions(&s.flush(&mut link, 13)),
        ["doc a", "place a", "doc b", "place b"]
    );
}

// A host that loses its documents loses them all at once, and the program
// sends its resources again once: the first ENOENT reports the loss, and
// every document goes again, the first numbered. An ENOENT read before that
// one's answer is for a placement sent before it: the same loss's.
#[test]
fn a_loss_is_reported_once() {
    let (mut s, mut link) = native();
    let all = || [want("a", 0, 0), want("b", 0, 3), want("c", 0, 6)];
    s.layout(all());
    s.flush(&mut link, 10);
    link.bytes(&gone("a"), 11);
    assert_eq!(
        s.poll(&mut link, 11),
        Some(SessionMessage::Relayout { lost: true })
    );
    link.bytes(&gone("b"), 11);
    assert_eq!(
        s.poll(&mut link, 11),
        None,
        "b's ENOENT before the documents went"
    );
    s.layout(all());
    let out = s.flush(&mut link, 12);
    assert_eq!(
        actions(&out),
        ["doc a", "place a", "doc b", "place b", "doc c", "place c"]
    );
    assert_eq!(out.matches(":n=").count(), 1, "documents numbered: {out:?}");
    let m = mark(&out).expect("a document numbered");
    assert_eq!(m.1, "a");
    link.bytes(&gone("c"), 13);
    assert_eq!(
        s.poll(&mut link, 13),
        None,
        "c's ENOENT before the mark's answer"
    );
    link.bytes(&marked(&m), 14);
    assert_eq!(s.poll(&mut link, 14), None, "the mark's answer");
    // The placements after the erase went after the mark: a loss of its
    // own.
    s.watch(b"\x1b[2J");
    assert_eq!(
        s.poll(&mut link, 15),
        Some(SessionMessage::Relayout { lost: false })
    );
    s.layout(all());
    assert_eq!(
        actions(&s.flush(&mut link, 16)),
        ["place a", "place b", "place c"]
    );
    link.bytes(&gone("b"), 17);
    assert_eq!(
        s.poll(&mut link, 17),
        Some(SessionMessage::Relayout { lost: true }),
        "a second loss"
    );
    link.bytes(&gone("a"), 18);
    assert_eq!(s.poll(&mut link, 18), None, "a's ENOENT in the second loss");
}

// Two frames in flight when the host loses its documents (margin's case,
// behind a relay): the second frame's ENOENTs come after the documents went
// again, and before the mark's answer, so they are still the first loss's.
#[test]
fn a_loss_behind_two_frames_is_reported_once() {
    let (mut s, mut link) = native();
    let at = |y| [want("a", 0, y), want("b", 0, y + 3)];
    s.layout(at(0));
    s.flush(&mut link, 10);
    s.layout(at(1));
    assert_eq!(actions(&s.flush(&mut link, 11)), ["place a", "place b"]);
    link.bytes(&gone("a"), 12);
    assert_eq!(
        s.poll(&mut link, 12),
        Some(SessionMessage::Relayout { lost: true })
    );
    link.bytes(&gone("b"), 12);
    assert_eq!(s.poll(&mut link, 12), None);
    s.layout(at(1));
    let m = mark(&s.flush(&mut link, 13)).expect("a document numbered");
    for name in ["a", "b"] {
        link.bytes(&gone(name), 14);
        assert_eq!(s.poll(&mut link, 14), None, "{name}'s second ENOENT");
    }
    link.bytes(&marked(&m), 15);
    assert_eq!(s.poll(&mut link, 15), None, "the mark's answer");
    link.bytes(&gone("a"), 16);
    assert_eq!(
        s.poll(&mut link, 16),
        Some(SessionMessage::Relayout { lost: true }),
        "an ENOENT after the mark's answer"
    );
}

// A mark whose answer never comes (the host that would have sent it went
// away) holds for 3 s from its flush; an ENOENT after that is a loss again.
#[test]
fn a_lost_mark_times_out() {
    let (mut s, mut link) = native();
    s.layout([want("a", 0, 0)]);
    s.flush(&mut link, 10);
    link.bytes(&gone("a"), 11);
    assert_eq!(
        s.poll(&mut link, 11),
        Some(SessionMessage::Relayout { lost: true })
    );
    s.layout([want("a", 0, 0)]);
    assert!(
        mark(&s.flush(&mut link, 20)).is_some(),
        "a's document numbered"
    );
    link.bytes(&gone("a"), 3019);
    assert_eq!(s.poll(&mut link, 3019), None, "an ENOENT inside the mark");
    link.bytes(&gone("a"), 3020);
    assert_eq!(
        s.poll(&mut link, 3020),
        Some(SessionMessage::Relayout { lost: true }),
        "an ENOENT past the mark"
    );
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
