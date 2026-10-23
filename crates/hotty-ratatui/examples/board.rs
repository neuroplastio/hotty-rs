//! Board is a full-screen ratatui program with a surface among its cells: a
//! counter with buttons and a form, in HTML on a hotty host, and in cells
//! anywhere else, beside a log of what the program heard.
//!
//! It shows the shape of a ratatui program on hotty-ratatui: draw with
//! `session.draw`, render a `Surface` where the HTML goes, read events with
//! `session.read`, and change the surface with deltas between frames.
//!
//!     cargo run -p hotty-ratatui --example board
//!
//! `+` and `-` change the count in either rendition; on a host, so do the
//! buttons, and the form adds a note. Escape takes the keyboard back from
//! the surface, and `q` quits.

use std::collections::VecDeque;

use hotty_ratatui::{Event, Mode, Session, Surface};
use ratatui::crossterm::event::{Event as Input, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, Paragraph};

const CARD: &str = r#"<style>
  body  { margin: 0; padding: 8px 12px; font: 14px system-ui, sans-serif; }
  .row  { display: flex; align-items: center; gap: 10px; }
  #count { font-size: 28px; min-width: 3ch; text-align: center; font-variant-numeric: tabular-nums; }
  button { font-size: 16px; padding: 2px 12px; }
  form  { margin-top: 10px; display: flex; gap: 6px; }
  input { flex: 1; }
</style>
<div class="row">
  <button id="dec">−</button><span id="count">COUNT</span><button id="inc">+</button>
  <span>clicks reach the program as events</span>
</div>
<form id="note"><input name="text" placeholder="a note for the log"><button>Add</button></form>"#;

struct App {
    count: i64,
    log: VecDeque<String>,
}

impl App {
    fn note(&mut self, s: impl Into<String>) {
        self.log.push_front(s.into());
        self.log.truncate(50);
    }
}

fn main() -> std::io::Result<()> {
    let (mut terminal, session) = hotty_ratatui::init("board")?;
    let result = run(&mut terminal, &session);
    hotty_ratatui::restore(&mut terminal, &session)?;
    result
}

fn run(terminal: &mut hotty_ratatui::DefaultTerminal, session: &Session) -> std::io::Result<()> {
    let card = session.surface("card");
    let mut app = App {
        count: 0,
        log: VecDeque::new(),
    };
    loop {
        let mode = session.mode();
        let count = app.count;
        session.draw(terminal, |frame, surfaces| {
            let [title, body, help] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .areas(frame.area());
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .areas(body);
            let rendition = match mode {
                Mode::Native => "a hotty host: HTML",
                Mode::Text => "not a host: cells",
                Mode::Detecting => "detecting",
            };
            frame.render_widget(
                Line::from(vec![
                    " board ".bold().reversed(),
                    format!(" {rendition}").into(),
                ]),
                title,
            );
            let block = Block::bordered().title(" surface ");
            let inner = block.inner(left);
            frame.render_widget(block, left);
            frame.render_stateful_widget(
                Surface::new(&card, || CARD.replace("COUNT", &count.to_string())).fallback(
                    Paragraph::new(vec![
                        Line::from(format!("count: {count}")).bold(),
                        Line::from("+ and - change it"),
                    ]),
                ),
                inner,
                surfaces,
            );
            frame.render_widget(
                List::new(app.log.iter().map(String::as_str))
                    .block(Block::bordered().title(" heard ")),
                right,
            );
            frame.render_widget(
                Line::from(" q quits · + − change the count").style(Style::new().dim()),
                help,
            );
        })?;

        match session.read()? {
            Event::Input(Input::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('+') => app.count += 1,
                KeyCode::Char('-') => app.count -= 1,
                // A surface keeps the keyboard until the program takes it
                // back (SPEC §10.1); Escape, which no field takes, does.
                KeyCode::Esc => session.send(hotty::blur(&card)),
                other => app.note(format!("key {other}")),
            },
            Event::Input(Input::Resize(c, r)) => app.note(format!("resized to {c}×{r}")),
            Event::Input(other) => app.note(format!("{other:?}")),
            Event::Ready(m) => app.note(format!("ready: {m:?}")),
            Event::Hotty(e) => {
                match (e.kind.as_str(), e.target.as_str()) {
                    ("click", "inc") => app.count += 1,
                    ("click", "dec") => app.count -= 1,
                    ("submit", _) => {
                        for (name, value) in e.fields.iter().flatten() {
                            app.note(format!("note: {name}={value}"));
                        }
                    }
                    _ => {}
                }
                app.note(format!("{} {}", e.kind, e.target));
            }
            Event::Error(r) => app.note(format!("error: {:?}", r.err())),
            Event::Relayout { .. } | Event::Ack(_) | Event::Pong(_) => {}
        }
        // The count changes in place: one delta, with the next frame.
        session.send(hotty::set_text(&card, "count", app.count));
    }
}
