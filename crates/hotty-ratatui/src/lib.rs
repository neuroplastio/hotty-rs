//! hotty (HTML over the TTY) for ratatui programs: surfaces as widgets,
//! and cells wherever the terminal is not a host.
//!
//! A hotty host is a terminal that shows small HTML documents, surfaces,
//! on rectangles of cells. With this crate a ratatui program renders a
//! [`Surface`] widget where it wants one: on a host the cells stay blank and
//! the host draws the document above them; anywhere else the widget renders
//! its fallback, an ordinary ratatui widget. What the user does in a
//! surface comes back as an [`Event`], beside crossterm's own.
//!
//! ```no_run
//! use hotty_ratatui::{Event, Surface};
//! use ratatui::crossterm::event::{Event as Input, KeyCode};
//! use ratatui::widgets::Paragraph;
//!
//! let (mut terminal, session) = hotty_ratatui::init("hello")?;
//! let card = session.surface("card");
//! loop {
//!     session.draw(&mut terminal, |frame, surfaces| {
//!         let area = frame.area();
//!         frame.render_stateful_widget(
//!             Surface::new(&card, || "<p>Hello, <b>HTML</b>. <button id=ok>OK</button></p>".into())
//!                 .fallback(Paragraph::new("Hello, cells. Press q.")),
//!             area,
//!             surfaces,
//!         );
//!     })?;
//!     match session.read()? {
//!         Event::Input(Input::Key(k)) if k.code == KeyCode::Char('q') => break,
//!         Event::Hotty(e) if e.kind == "click" => break,
//!         _ => {} // Ready, Relayout, a resize: draw again
//!     }
//! }
//! hotty_ratatui::restore(&mut terminal, &session)?;
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! # Input
//!
//! The host answers on the terminal's input, among the keys, and
//! crossterm's reader would take its answers for keys. So the [`Session`]
//! reads the terminal itself, cuts hotty's sequences out, and hands the rest
//! back as crossterm's events ([`Event::Input`], parsed by [`input::Parser`]):
//! a program reads [`Session::read`] and [`Session::poll`] where it read
//! crossterm's `event::read` and `event::poll`, and keeps its handlers. It
//! must not read the terminal any other way while the session is open.
//!
//! # Output
//!
//! [`init`] gives a ratatui `Terminal` on this crate's [`Backend`], which
//! writes each frame with the session's commands in it, and watches the
//! frames for what drops placements (an erase, a scroll). A program that
//! makes its own terminal gives it a backend on a [`Writer`]; with any other
//! backend, [`Session::draw`] writes the commands after the frame.
#![cfg(unix)]

mod backend;
pub mod input;
mod session;
mod surface;

use std::io;
use std::sync::{Arc, Weak};
use std::time::Duration;

pub use backend::{Backend, Writer};
pub use hotty;
pub use hotty::session::Mode;
pub use session::{Builder, Event, Session};
pub use surface::{Surface, Surfaces};

/// The terminal [`init`] makes.
pub type DefaultTerminal = ratatui_core::terminal::Terminal<Backend>;

/// Opens the process's terminal for a full-screen program: a [`Session`]
/// (raw mode, the input read, detection started), the alternate screen, and
/// a ratatui terminal on a [`Backend`] of the session's. It waits until
/// detection decides, so that the first frame draws the right rendition: at
/// once on a host, 150 ms on a terminal that answers only DA1, 1.5 s at
/// most. A panic restores the terminal before it reports.
pub fn init(name: &str) -> io::Result<(DefaultTerminal, Session)> {
    init_with(Session::builder(name))
}

/// [`init`], with a session built as `builder` says.
pub fn init_with(builder: Builder) -> io::Result<(DefaultTerminal, Session)> {
    let session = builder.open()?;
    session.terminal().write(b"\x1b[?1049h")?;
    set_panic_hook(Arc::downgrade(&session.inner));
    session.wait_ready(Duration::from_millis(hotty::DETECT_TIMEOUT))?;
    let terminal = ratatui_core::terminal::Terminal::new(Backend::new(&session))?;
    Ok((terminal, session))
}

/// Gives the terminal back: deletes the session's surfaces, shows the
/// cursor, leaves the alternate screen, and closes the session.
pub fn restore(terminal: &mut DefaultTerminal, session: &Session) -> io::Result<()> {
    let r = session.close();
    terminal.show_cursor()?;
    session.terminal().write(b"\x1b[?1049l")?;
    r
}

fn set_panic_hook(inner: Weak<session::Inner>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(inner) = inner.upgrade() {
            let s = Session { inner };
            let _ = s.close();
            let _ = s.terminal().write(b"\x1b[?25h\x1b[?1049l");
        }
        previous(info);
    }));
}
