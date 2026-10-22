//! The Terminal, against a fake host over pipes (SDK.md §4.1).
#![cfg(all(unix, feature = "term"))]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hotty::link::Input;
use hotty::{Control, Decoder, Fed, Placement, Scanner, Segment, Terminal, encode_plain};

// {"v": "0.5"}, as a host writes it.
const CAPS_05: &[u8] = &[0x81, 0xa1, b'v', 0xa3, b'0', b'.', b'5'];

/// A host that answers the query, numbered commands, DA1 and CPR, and
/// records every command it took.
struct FakeHost {
    commands: Arc<Mutex<Vec<Control>>>,
    to_program: Arc<Mutex<std::io::PipeWriter>>,
}

impl FakeHost {
    fn start(native: bool) -> (FakeHost, Terminal) {
        let (from_host, to_program) = std::io::pipe().unwrap();
        let (from_program, to_host) = std::io::pipe().unwrap();
        let t = Terminal::builder("t").size(100, 30).io(from_host, to_host);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let to_program = Arc::new(Mutex::new(to_program));
        let (cmds, out) = (commands.clone(), to_program.clone());
        thread::spawn(move || serve(from_program, out, cmds, native));
        (
            FakeHost {
                commands,
                to_program,
            },
            t,
        )
    }

    fn type_keys(&self, b: &[u8]) {
        self.to_program.lock().unwrap().write_all(b).unwrap();
    }

    fn actions(&self) -> Vec<String> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.action().to_string())
            .collect()
    }
}

fn serve(
    mut input: std::io::PipeReader,
    out: Arc<Mutex<std::io::PipeWriter>>,
    cmds: Arc<Mutex<Vec<Control>>>,
    native: bool,
) {
    let mut scanner = Scanner::new();
    let mut decoder = Decoder::new();
    let mut buf = [0u8; 4096];
    loop {
        let Ok(n) = input.read(&mut buf) else { return };
        if n == 0 {
            return;
        }
        for seg in scanner.feed(&buf[..n]) {
            let answer: Vec<u8> = match seg {
                Segment::Pass(b) => {
                    let mut a = Vec::new();
                    let s = String::from_utf8_lossy(&b).to_string();
                    let mut rest = s.as_str();
                    while let Some(i) = rest.find('\x1b') {
                        rest = &rest[i..];
                        if rest.starts_with("\x1b[c") {
                            a.extend_from_slice(b"\x1b[?62;22c");
                        } else if rest.starts_with("\x1b[6n") {
                            a.extend_from_slice(b"\x1b[5;7R");
                        }
                        rest = &rest[1..];
                    }
                    a
                }
                Segment::Osc(seq) if native => match decoder.feed(&seq) {
                    Fed::Complete(m) => {
                        cmds.lock().unwrap().push(m.control.clone());
                        reply_to(&m.control)
                    }
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            if !answer.is_empty() {
                out.lock().unwrap().write_all(&answer).unwrap();
            }
        }
    }
}

fn reply_to(c: &Control) -> Vec<u8> {
    let a = c.action();
    if a == "q" {
        if c.get("v") == Some("0.5") {
            let ctl = Control::from([("a", "ok"), ("n", c.get("n").unwrap()), ("re", "q")]);
            return encode_plain(&ctl, CAPS_05).into_bytes();
        }
        return Vec::new();
    }
    let mut ctl = Control::new();
    if c.get("s") == Some("t-bad") {
        ctl = ctl.with("a", "err");
    } else if let Some(n) = c.get("n") {
        ctl = ctl.with("a", "ok").with("n", n);
    } else {
        return Vec::new();
    }
    ctl = ctl.with("re", a);
    if let Some(s) = c.get("s") {
        ctl = ctl.with("s", s);
    }
    if a == "place" {
        ctl = ctl.with("c", c.get("c").unwrap()).with("r", "3");
    }
    encode_plain(&ctl, b"").into_bytes()
}

#[test]
fn a_host_is_detected_and_answers_requests() {
    let (host, t) = FakeHost::start(true);
    assert!(t.detect());
    assert_eq!(
        t.caps().and_then(|c| c.v),
        Some("0.5".to_string()),
        "the host's capabilities"
    );
    let name = t.surface("card");
    t.send(hotty::doc(&name, "<p>hi</p>")).unwrap();
    let r = t
        .request(hotty::place(&name, &Placement::new(40, 0)))
        .unwrap();
    assert!(r.ok);
    assert_eq!(r.rows, Some(3));
    assert_eq!(r.surface.as_deref(), Some("t-card"));
    assert_eq!(host.actions(), ["q", "doc", "place"]);
}

#[test]
fn a_fence_returns_the_errors_nothing_read() {
    let (_host, t) = FakeHost::start(true);
    assert!(t.detect());
    t.print("bad", "<p>x</p>", &Placement::new(10, 1)).unwrap();
    let replies = t.fence().unwrap();
    assert_eq!(replies.len(), 2, "the doc's error and the place's");
    assert!(replies.iter().all(|r| !r.ok));
}

#[test]
fn a_terminal_that_is_not_a_host_answers_only_da1() {
    let (_host, t) = FakeHost::start(false);
    let started = std::time::Instant::now();
    assert!(!t.detect());
    assert!(
        started.elapsed() >= Duration::from_millis(140),
        "the grace after DA1"
    );
}

#[test]
fn keys_reach_the_input_stream_and_the_cursor_is_asked_for() {
    let (host, t) = FakeHost::start(true);
    assert!(t.detect());
    t.line_start();
    host.type_keys(b"ab");
    assert_eq!(
        t.next_input(Some(Duration::from_secs(2))),
        Some(Input::Data(b"ab".to_vec()))
    );
    assert_eq!(t.next_input(Some(Duration::from_millis(20))), None);
}

#[test]
fn closing_ends_the_input_stream() {
    let (_host, t) = FakeHost::start(true);
    assert!(t.detect());
    t.close();
    assert_eq!(t.next_input(Some(Duration::from_millis(20))), None);
}
