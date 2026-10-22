//! The Link: what it routes where, sans I/O (SDK.md §4.1, §4.2).

use hotty::link::{Answer, Input, Known, Link};
use hotty::{Caps, Control, Placement, doc, encode_plain, place};

// {"v": "0.5"}, as a host writes it.
const CAPS_05: &[u8] = &[0x81, 0xa1, b'v', 0xa3, b'0', b'.', b'5'];

fn reply(pairs: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    encode_plain(&pairs.iter().copied().collect::<Control>(), body).into_bytes()
}

fn host_answer() -> Vec<u8> {
    let mut b = reply(&[("a", "ok"), ("n", "1"), ("re", "q")], CAPS_05);
    b.extend_from_slice(b"\x1b[?62;22c");
    b
}

fn native() -> Link {
    let mut l = Link::new("t");
    l.detect(0, false);
    l.take_output();
    l.bytes(&host_answer(), 5);
    assert!(l.native() && l.detected());
    l
}

#[test]
fn a_host_is_decided_by_its_reply_and_done_by_the_da1_behind_it() {
    let mut l = Link::new("t");
    l.detect(0, false);
    let q = l.take_output();
    assert!(q.starts_with(b"\x1b]7279;a=q:n=1:v=0.5\x1b\\"));
    assert!(l.waiting());
    l.bytes(&reply(&[("a", "ok"), ("n", "1"), ("re", "q")], CAPS_05), 10);
    assert!(l.decided() && l.native() && !l.detected());
    assert_eq!(l.caps().and_then(|c| c.v.as_deref()), Some("0.5"));
    l.bytes(b"\x1b[?62;22c", 12);
    assert!(l.detected());
    assert!(!l.waiting());
    assert_eq!(l.poll_input(), None, "detection swallows its answers");
}

#[test]
fn silence_is_not_a_host() {
    let mut l = Link::new("t");
    l.detect(0, false);
    assert_eq!(l.deadline(), Some(1500));
    l.tick(1499);
    assert!(!l.decided());
    l.tick(1500);
    assert!(l.decided() && l.detected() && !l.native());
}

#[test]
fn a_known_terminal_answers_at_once() {
    let mut l = Link::new("t").known(Known {
        native: true,
        caps: Some(Caps::default()),
    });
    l.detect(0, false);
    assert!(l.take_output().is_empty());
    assert!(l.native() && l.detected());
}

#[test]
fn a_request_gets_its_numbered_reply_and_the_rest_goes_to_the_stream() {
    let mut l = native();
    let t = l.request(place("s", &Placement::new(40, 0)), 10);
    let out = String::from_utf8(l.take_output()).unwrap();
    assert!(out.contains("a=place:s=s:c=40:r=auto:n=2"), "{out}");
    l.bytes(b"a", 11);
    l.bytes(
        &reply(
            &[
                ("a", "ok"),
                ("n", "2"),
                ("re", "place"),
                ("c", "40"),
                ("r", "3"),
            ],
            b"",
        ),
        12,
    );
    let Some(Ok(Answer::Reply(r))) = l.answer(t) else {
        panic!("no reply")
    };
    assert_eq!((r.ok, r.rows), (true, Some(3)));
    assert_eq!(l.poll_input(), Some(Input::Data(b"a".to_vec())));
    assert_eq!(l.poll_input(), None);
}

#[test]
fn a_request_without_a_reply_gives_up() {
    let mut l = native();
    let t = l.request(doc("s", "x"), 10);
    l.tick(3009);
    assert!(l.answer(t).is_none());
    l.tick(3010);
    assert!(matches!(l.answer(t), Some(Err(_))));
}

#[test]
fn a_fence_takes_the_unnumbered_replies_before_its_da1() {
    let mut l = native();
    l.bytes(&reply(&[("a", "err"), ("re", "doc"), ("s", "a")], b""), 6);
    let t = l.fence(7);
    assert!(l.take_output().ends_with(b"\x1b[c"));
    l.bytes(&reply(&[("a", "err"), ("re", "place"), ("s", "b")], b""), 8);
    l.bytes(b"\x1b[?62;22c", 9);
    let Some(Ok(Answer::Fence(replies))) = l.answer(t) else {
        panic!("no fence")
    };
    let res: Vec<&str> = replies.iter().map(|r| r.re.as_str()).collect();
    assert_eq!(res, ["doc", "place"]);
    assert_eq!(l.poll_input(), None);
}

#[test]
fn the_cursor_position_is_cut_out_of_the_keys_around_it() {
    let mut l = native();
    let t = l.cursor_position(10);
    assert_eq!(l.take_output(), b"\x1b[6n");
    l.bytes(b"x\x1b[12;", 11);
    assert!(l.answer(t).is_none());
    l.bytes(b"5Ry", 12);
    assert_eq!(l.answer(t), Some(Ok(Answer::Cursor { row: 11, col: 4 })));
    assert_eq!(l.poll_input(), Some(Input::Data(b"x".to_vec())));
    assert_eq!(l.poll_input(), Some(Input::Data(b"y".to_vec())));
}

#[test]
fn a_lone_esc_is_the_escape_key_after_the_delay() {
    let mut l = native();
    l.bytes(b"\x1b", 100);
    assert_eq!(l.poll_input(), None);
    assert_eq!(l.deadline(), Some(125));
    l.tick(125);
    assert_eq!(l.poll_input(), Some(Input::Data(b"\x1b".to_vec())));
}

#[test]
fn an_esc_that_begins_a_hotty_sequence_waits_for_it() {
    let mut l = native();
    let ev = reply(&[("a", "ev"), ("s", "t-x"), ("e", "press")], b"");
    l.bytes(&ev[..1], 100);
    l.bytes(&ev[1..], 110);
    let Some(Input::Message(m)) = l.poll_input() else {
        panic!("no event")
    };
    assert_eq!(m.event().unwrap().kind, "press");
}

#[test]
fn a_late_answer_makes_the_terminal_a_host() {
    let mut l = Link::new("t").late(true);
    l.detect(0, false);
    assert!(
        String::from_utf8(l.take_output())
            .unwrap()
            .contains("late=1")
    );
    l.bytes(b"\x1b[?62;22c", 10);
    l.tick(160);
    assert!(l.detected() && !l.native());
    assert!(l.waiting(), "a late answer may still come");
    l.bytes(
        &reply(&[("a", "ok"), ("n", "1"), ("re", "q")], CAPS_05),
        5000,
    );
    assert!(l.native());
    assert!(matches!(l.poll_input(), Some(Input::Host(Some(_)))));
    assert!(!l.waiting());
}

#[test]
fn closing_withdraws_a_late_query_no_answer_came_to() {
    let mut l = Link::new("t").late(true);
    l.detect(0, false);
    l.take_output();
    l.tick(1500);
    l.close(1600);
    assert_eq!(l.take_output(), b"\x1b]7279;a=q:q=2\x1b\\");
    // An answer after it reaches nothing.
    l.bytes(
        &reply(&[("a", "ok"), ("n", "1"), ("re", "q")], CAPS_05),
        1700,
    );
    assert!(!l.native());
}

#[test]
fn surfaces_are_named_the_programs_own() {
    let mut l = Link::new("tool 42");
    assert_eq!(l.surface("card"), "tool_42-card");
    l.surface("card");
    l.surface("chart");
    assert_eq!(l.surfaces(), ["tool_42-card", "tool_42-chart"]);
    l.detach_all();
    let out = String::from_utf8(l.take_output()).unwrap();
    assert_eq!(out.matches("a=detach").count(), 2);
}

#[test]
fn a_ping_times_the_da1() {
    let mut l = native();
    let t = l.ping(100);
    assert_eq!(l.take_output(), b"\x1b[c");
    l.bytes(b"\x1b[?62;22c", 130);
    assert_eq!(l.answer(t), Some(Ok(Answer::Da1 { took: 30 })));
}

#[test]
fn kitty_graphics_answers_and_takes_the_da1_behind() {
    let mut l = native();
    let t = l.kitty_graphics(0);
    l.take_output();
    l.bytes(b"\x1b_Gi=31;OK\x1b\\", 5);
    assert!(l.answer(t).is_none());
    l.bytes(b"\x1b[?62;22c", 6);
    assert_eq!(l.answer(t), Some(Ok(Answer::Graphics(true))));
    assert_eq!(l.poll_input(), None);
}
