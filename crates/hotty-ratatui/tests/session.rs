//! The ratatui session, end to end against a fake host over pipes: frames
//! carry their surfaces' commands, keys come back as crossterm's events, and
//! events from surfaces as hotty's.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hotty::{Control, Decoder, Fed, Scanner, Segment, encode_plain};
use hotty_ratatui::{Backend, Event, Mode, Session, Surface};
use ratatui::Terminal;
use ratatui::crossterm::event::{Event as Input, KeyCode};
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

const CAPS_05: &[u8] = &[0x81, 0xa1, b'v', 0xa3, b'0', b'.', b'5'];

struct Host {
    commands: Arc<Mutex<Vec<Control>>>,
    raw: Arc<Mutex<Vec<u8>>>,
    to_program: Arc<Mutex<std::io::PipeWriter>>,
}

impl Host {
    fn start(native: bool) -> (Host, Session) {
        let (from_host, to_program) = std::io::pipe().unwrap();
        let (mut from_program, to_host) = std::io::pipe().unwrap();
        let term = hotty::Terminal::builder("app")
            .size(40, 10)
            .io(from_host, to_host);
        let host = Host {
            commands: Arc::default(),
            raw: Arc::default(),
            to_program: Arc::new(Mutex::new(to_program)),
        };
        let (cmds, raw, out) = (
            host.commands.clone(),
            host.raw.clone(),
            host.to_program.clone(),
        );
        thread::spawn(move || {
            let (mut sc, mut dec) = (Scanner::new(), Decoder::new());
            let mut buf = [0u8; 8192];
            while let Ok(n) = from_program.read(&mut buf) {
                if n == 0 {
                    return;
                }
                raw.lock().unwrap().extend_from_slice(&buf[..n]);
                for seg in sc.feed(&buf[..n]) {
                    let answer = match seg {
                        Segment::Pass(b) => {
                            let mut a = Vec::new();
                            if b.windows(4).any(|w| w == b"\x1b[6n") {
                                a.extend_from_slice(b"\x1b[1;1R");
                            }
                            if b.windows(3).any(|w| w == b"\x1b[c") {
                                a.extend_from_slice(b"\x1b[?62;22c");
                            }
                            a
                        }
                        Segment::Osc(s) if native => match dec.feed(&s) {
                            Fed::Complete(m) => {
                                cmds.lock().unwrap().push(m.control.clone());
                                if m.action() == "q" {
                                    let c = Control::from([("a", "ok"), ("n", "1"), ("re", "q")]);
                                    encode_plain(&c, CAPS_05).into_bytes()
                                } else {
                                    Vec::new()
                                }
                            }
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    };
                    out.lock().unwrap().write_all(&answer).unwrap();
                }
            }
        });
        let session = Session::on(term).unwrap();
        (host, session)
    }

    fn send(&self, b: &[u8]) {
        self.to_program.lock().unwrap().write_all(b).unwrap();
    }

    fn actions(&self) -> Vec<String> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.action() != "q")
            .map(|c| format!("{} {}", c.action(), c.get("s").unwrap_or("")))
            .collect()
    }

    fn wait_for(&self, n: usize) {
        for _ in 0..200 {
            if self.actions().len() >= n {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("the host has {:?}", self.actions());
    }
}

fn draw(session: &Session, terminal: &mut Terminal<Backend>, name: &str) {
    session
        .draw(terminal, |frame, surfaces| {
            frame.render_widget(Paragraph::new("cells"), Rect::new(0, 0, 40, 1));
            frame.render_stateful_widget(
                Surface::new(name, || "<p>hi</p>".to_string()).fallback(Paragraph::new("fallback")),
                Rect::new(2, 3, 20, 4),
                surfaces,
            );
        })
        .unwrap();
}

#[test]
fn on_a_host_a_surface_goes_out_with_its_frame() {
    let (host, session) = Host::start(true);
    assert_eq!(
        session.wait_ready(Duration::from_secs(2)).unwrap(),
        Mode::Native
    );
    assert_eq!(session.read().unwrap(), Event::Ready(Mode::Native));
    let mut terminal = Terminal::new(Backend::new(&session)).unwrap();
    let card = session.surface("card");
    draw(&session, &mut terminal, &card);
    host.wait_for(2);
    assert_eq!(
        host.actions(),
        [format!("doc {card}"), format!("place {card}")]
    );
    let raw = String::from_utf8_lossy(&host.raw.lock().unwrap()).to_string();
    assert!(
        raw.contains("\x1b7\x1b[4;3H\x1b]7279;a=place"),
        "placed where it rendered"
    );
    assert_eq!(
        ratatui::backend::Backend::size(terminal.backend()).unwrap(),
        ratatui::layout::Size::new(40, 10)
    );

    draw(&session, &mut terminal, &card);
    thread::sleep(Duration::from_millis(30));
    assert_eq!(host.actions().len(), 2, "an unchanged frame sends nothing");

    session.draw(&mut terminal, |_frame, _surfaces| {}).unwrap();
    host.wait_for(3);
    assert_eq!(host.actions()[2], format!("del {card}"));
}

#[test]
fn keys_come_back_as_crosstermss_events_and_clicks_as_hottys() {
    let (host, session) = Host::start(true);
    assert_eq!(session.read().unwrap(), Event::Ready(Mode::Native));
    host.send(b"q\x1b[A");
    let ev = encode_plain(
        &Control::from([("a", "ev"), ("s", "app-card"), ("e", "click"), ("t", "ok")]),
        b"",
    );
    host.send(ev.as_bytes());
    let Event::Input(Input::Key(k)) = session.read().unwrap() else {
        panic!("not a key")
    };
    assert_eq!(k.code, KeyCode::Char('q'));
    let Event::Input(Input::Key(k)) = session.read().unwrap() else {
        panic!("not a key")
    };
    assert_eq!(k.code, KeyCode::Up);
    let Event::Hotty(e) = session.read().unwrap() else {
        panic!("not an event")
    };
    assert_eq!((e.kind.as_str(), e.target.as_str()), ("click", "ok"));
    assert!(!session.poll(Duration::from_millis(30)).unwrap());
}

#[test]
fn a_lone_escape_is_the_escape_key() {
    let (host, session) = Host::start(true);
    assert_eq!(session.read().unwrap(), Event::Ready(Mode::Native));
    host.send(b"\x1b");
    let Event::Input(Input::Key(k)) = session.read().unwrap() else {
        panic!("not a key")
    };
    assert_eq!(k.code, KeyCode::Esc);
}

#[test]
fn on_a_terminal_that_is_not_a_host_the_fallback_draws() {
    let (host, session) = Host::start(false);
    assert_eq!(
        session.wait_ready(Duration::from_secs(2)).unwrap(),
        Mode::Text
    );
    let mut terminal = Terminal::new(Backend::new(&session)).unwrap();
    let card = session.surface("card");
    draw(&session, &mut terminal, &card);
    thread::sleep(Duration::from_millis(30));
    assert!(
        host.actions().is_empty(),
        "nothing of hotty's but the query"
    );
    let raw = String::from_utf8_lossy(&host.raw.lock().unwrap()).to_string();
    assert!(raw.contains("fallback"), "{raw:?}");
}

#[test]
fn an_erase_asks_for_a_relayout() {
    let (host, session) = Host::start(true);
    assert_eq!(session.read().unwrap(), Event::Ready(Mode::Native));
    let mut terminal = Terminal::new(Backend::new(&session)).unwrap();
    let card = session.surface("card");
    draw(&session, &mut terminal, &card);
    terminal.clear().unwrap();
    assert_eq!(session.read().unwrap(), Event::Relayout { lost: false });
    draw(&session, &mut terminal, &card);
    host.wait_for(3);
    assert_eq!(host.actions()[2], format!("place {card}"));
}

#[test]
fn closing_deletes_the_surfaces() {
    let (host, session) = Host::start(true);
    assert_eq!(session.read().unwrap(), Event::Ready(Mode::Native));
    let mut terminal = Terminal::new(Backend::new(&session)).unwrap();
    let card = session.surface("card");
    draw(&session, &mut terminal, &card);
    session.close().unwrap();
    host.wait_for(3);
    assert_eq!(host.actions()[2], format!("del {card}"));
}
