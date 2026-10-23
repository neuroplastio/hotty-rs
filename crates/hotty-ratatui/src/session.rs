//! The program's side of hotty, on the process's terminal (SDK.md §4.3).

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use ratatui_core::backend::Backend as RatatuiBackend;
use ratatui_core::terminal::{Frame, Terminal};
use ratatui_crossterm::crossterm::event::Event as CrosstermEvent;

use hotty::link::ESCAPE_DELAY;
use hotty::session::{Mode, SessionMessage};
use hotty::term::RawGuard;
use hotty::{Caps, Reply};

use crate::input::Parser;
use crate::surface::Surfaces;

/// What reaches the program: the terminal's own input, as crossterm reads
/// it, and what the host says.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // passed now and then, not stored in bulk
pub enum Event {
    /// Input that is not hotty's, as crossterm reads it: keys, the mouse,
    /// pastes, focus, and resizes. A program hands it to the handler it has.
    Input(CrosstermEvent),
    /// Detection decided what the terminal is; again, native, when a late
    /// answer comes. Draw a frame: surfaces render as themselves, or as
    /// their fallback cells.
    Ready(Mode),
    /// Something the user did in a surface (SPEC §9).
    Hotty(hotty::Event),
    /// A command the host refused that the session does not handle itself.
    Error(Reply),
    /// The host's ok for a command the program numbered ([`hotty::Cmd::n`]).
    Ack(Reply),
    /// The screen was erased or scrolled under the surfaces, or a document
    /// went missing: draw a frame, which places them again.
    Relayout {
        /// The host lost a document, and may have lost the resources
        /// ([`hotty::res`]) it refers to as well (SPEC §2, §7.1): send them
        /// again ([`Session::send`]) before drawing, so that they reach the
        /// host ahead of the document; sending one again replaces it. One
        /// loss is reported once, however many documents it took.
        lost: bool,
    },
    /// A ping's answer ([`Session::ping`]): the terminal took the frame
    /// written before it, this long after.
    Pong(Duration),
}

struct State {
    session: hotty::Session,
    parser: Parser,
    events: VecDeque<Event>,
    parser_at: Option<u64>,
    resized: bool,
}

pub(crate) struct Inner {
    pub(crate) term: hotty::Terminal,
    state: Mutex<State>,
    raw: Mutex<Option<RawGuard>>,
    signals: Mutex<Option<signal_hook::iterator::Handle>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(h) = self
            .signals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            h.close();
        }
    }
}

/// A ratatui program's side of hotty (SDK.md §4.3), on the process's
/// terminal: it finds out whether the terminal is a host, keeps the
/// program's surfaces on screen as its frames change, and reads the
/// terminal's input, handing it back as crossterm's events.
///
/// It reads the terminal itself, since crossterm's reader would take the
/// host's replies for keys: a program reads its events with
/// [`Session::read`] and [`Session::poll`], in place of crossterm's
/// `event::read` and `event::poll`, and must not call those, or anything
/// else that reads the terminal (`cursor::position`, say), while the session
/// is open. A clone is the same session.
#[derive(Clone)]
pub struct Session {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("mode", &self.mode())
            .finish_non_exhaustive()
    }
}

/// How a [`Session`] opens: [`Session::builder`].
#[derive(Debug)]
pub struct Builder {
    name: String,
    late: bool,
    limit: usize,
}

impl Builder {
    /// Ask for a late answer (SPEC §4): if one comes while the mode is text,
    /// the mode becomes native and [`Event::Ready`] comes again. A program
    /// that asks for it draws either rendition whenever the mode says.
    #[must_use]
    pub fn late(mut self, late: bool) -> Builder {
        self.late = late;
        self
    }

    /// The most surfaces to keep, shown and hidden
    /// ([`hotty::session::DEFAULT_LIMIT`] when 0).
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Builder {
        self.limit = limit;
        self
    }

    /// Opens the process's terminal, puts it in raw mode, and starts
    /// reading it and detecting whether it is a host. Fails when the
    /// process has no terminal.
    pub fn open(self) -> io::Result<Session> {
        let term = hotty::Terminal::builder(&self.name)
            .late(self.late)
            .open()?;
        self.on(term)
    }

    /// A session on a terminal opened another way, such as one made from
    /// streams for a test ([`hotty::term::Builder::io`]). Ask that terminal
    /// for a late answer too, when the session asks for one.
    pub fn on(self, term: hotty::Terminal) -> io::Result<Session> {
        Session::make(term, self.late, self.limit)
    }
}

impl Session {
    /// Opens the process's terminal: [`Builder::open`] with the defaults.
    pub fn open(name: &str) -> io::Result<Session> {
        Session::builder(name).open()
    }

    /// A builder for a session whose surfaces are named after `name`.
    pub fn builder(name: &str) -> Builder {
        Builder {
            name: name.to_string(),
            late: false,
            limit: 0,
        }
    }

    /// A session on a terminal opened another way: [`Builder::on`] with the
    /// defaults.
    pub fn on(term: hotty::Terminal) -> io::Result<Session> {
        Session::make(term, false, 0)
    }

    fn make(term: hotty::Terminal, late: bool, limit: usize) -> io::Result<Session> {
        let raw = term.raw();
        let session = hotty::Session::new().late(late).limit(limit);
        let s = Session {
            inner: Arc::new(Inner {
                term,
                state: Mutex::new(State {
                    session,
                    parser: Parser::new(),
                    events: VecDeque::new(),
                    parser_at: None,
                    resized: false,
                }),
                raw: Mutex::new(Some(raw)),
                signals: Mutex::new(None),
            }),
        };
        {
            let mut st = s.state();
            s.inner
                .term
                .with_link(|link, now| st.session.detect(link, now))?;
        }
        s.watch_resizes();
        Ok(s)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn watch_resizes(&self) {
        let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH])
        else {
            return;
        };
        *self.inner.signals.lock().unwrap_or_else(|e| e.into_inner()) = Some(signals.handle());
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let _ = std::thread::Builder::new()
            .name("hotty-resize".into())
            .spawn(move || {
                for _ in signals.forever() {
                    let Some(inner) = weak.upgrade() else { return };
                    inner
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .resized = true;
                    inner.term.wake();
                }
            });
    }

    /// What the terminal turned out to be: [`Mode::Detecting`] until
    /// detection decides.
    pub fn mode(&self) -> Mode {
        self.state().session.mode()
    }

    /// The host's capabilities, when native, kept current by the `caps`
    /// events it sends.
    pub fn caps(&self) -> Option<Caps> {
        self.state().session.caps().cloned()
    }

    /// The terminal under the session: for what it does that a session does
    /// not, such as [`hotty::Terminal::request`].
    pub fn terminal(&self) -> &hotty::Terminal {
        &self.inner.term
    }

    /// A surface name of this program's own: the program's name and process
    /// id, a dash, and `name`, made a valid surface name.
    pub fn surface(&self, name: &str) -> String {
        self.inner.term.surface(name)
    }

    // Reads what the link was handed into events, and lets the time pass.
    fn pump(&self, st: &mut State) -> io::Result<()> {
        if st.resized {
            st.resized = false;
            let (cols, rows) = self.inner.term.size();
            st.events
                .push_back(Event::Input(CrosstermEvent::Resize(cols, rows)));
        }
        let State {
            session,
            parser,
            events,
            parser_at,
            ..
        } = st;
        self.inner.term.with_link(|link, now| {
            while let Some(m) = session.poll(link, now) {
                match m {
                    SessionMessage::Input(b) => {
                        events.extend(parser.feed(&b).into_iter().map(Event::Input));
                        // A lone ESC the link held for its delay is the
                        // Escape key now.
                        if b == b"\x1b" && parser.holding() {
                            events.extend(parser.flush().into_iter().map(Event::Input));
                        }
                        *parser_at = (parser.holding() && !parser.in_sequence())
                            .then_some(now + ESCAPE_DELAY);
                    }
                    SessionMessage::Ready { mode, .. } => events.push_back(Event::Ready(mode)),
                    SessionMessage::Event(e) => events.push_back(Event::Hotty(e)),
                    SessionMessage::Error(r) => events.push_back(Event::Error(r)),
                    SessionMessage::Ack(r) => events.push_back(Event::Ack(r)),
                    SessionMessage::Relayout { lost } => {
                        events.push_back(Event::Relayout { lost });
                    }
                    SessionMessage::Pong { took } => {
                        events.push_back(Event::Pong(Duration::from_millis(took)));
                    }
                }
            }
            if parser_at.is_some_and(|at| now >= at) {
                *parser_at = None;
                events.extend(parser.flush().into_iter().map(Event::Input));
            }
        })
    }

    fn next_deadline(&self, st: &State) -> Option<u64> {
        match (st.parser_at, st.session.deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Whether an event is ready, waiting `timeout` at most for one: as
    /// crossterm's `event::poll`.
    pub fn poll(&self, timeout: Duration) -> io::Result<bool> {
        let term = &self.inner.term;
        let until = term.now() + timeout.as_millis() as u64;
        loop {
            let mut st = self.state();
            self.pump(&mut st)?;
            if !st.events.is_empty() {
                return Ok(true);
            }
            let now = term.now();
            if now >= until {
                return Ok(false);
            }
            if term.ended() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let at = self.next_deadline(&st).map_or(until, |d| d.min(until));
            drop(st);
            term.wait(Duration::from_millis(at.saturating_sub(now).max(1)));
        }
    }

    /// The next event, waiting for one: as crossterm's `event::read`. Fails
    /// once the terminal's input has ended.
    pub fn read(&self) -> io::Result<Event> {
        loop {
            if let Some(e) = self.try_read()? {
                return Ok(e);
            }
            self.poll(Duration::from_secs(3600))?;
        }
    }

    /// The next event, if one is ready.
    pub fn try_read(&self) -> io::Result<Option<Event>> {
        let mut st = self.state();
        if st.events.is_empty() {
            self.pump(&mut st)?;
        }
        Ok(st.events.pop_front())
    }

    /// Waits until detection decides what the terminal is, `timeout` at
    /// most, and returns the mode. The [`Event::Ready`] it produced stays
    /// for [`Session::read`].
    pub fn wait_ready(&self, timeout: Duration) -> io::Result<Mode> {
        let term = &self.inner.term;
        let until = term.now() + timeout.as_millis() as u64;
        loop {
            {
                let mut st = self.state();
                self.pump(&mut st)?;
                let mode = st.session.mode();
                if mode != Mode::Detecting || term.now() >= until {
                    return Ok(mode);
                }
            }
            if term.ended() {
                return Ok(self.mode());
            }
            term.wait(Duration::from_millis(20));
        }
    }

    /// Draws a frame, as `terminal.draw` does, with its surfaces:
    /// `render` renders widgets into the frame, and [`Surface`](crate::Surface)
    /// widgets into `surfaces` as well. Once the frame is drawn, the
    /// surfaces it rendered are on screen and the rest are hidden or
    /// deleted; their commands go out with the frame's cells.
    pub fn draw<'a, B, F>(&self, terminal: &mut Terminal<B>, render: F) -> Result<(), B::Error>
    where
        B: RatatuiBackend,
        B::Error: From<io::Error>,
        F: FnOnce(&mut Frame<'_>, &mut Surfaces<'a>),
    {
        let mode = self.mode();
        terminal.draw(|frame| {
            let mut surfaces = Surfaces::new(mode);
            render(frame, &mut surfaces);
            self.layout(surfaces.into_wants());
        })?;
        // A backend that is not this crate's writes nothing of hotty's.
        self.flush()?;
        Ok(())
    }

    pub(crate) fn layout<'a>(&self, wants: Vec<hotty::session::Want<'a>>) {
        self.state().session.layout(wants);
    }

    /// Queues commands (deltas, focus, a menu) for the next frame, to go out
    /// with its cells. Nothing when the terminal is not a host.
    pub fn send(&self, commands: impl std::fmt::Display) {
        self.state().session.send(commands);
    }

    /// Writes what is queued now, without waiting for a frame.
    pub fn flush(&self) -> io::Result<()> {
        let out = self.take_commands()?;
        if out.is_empty() {
            return Ok(());
        }
        self.inner.term.write(out.as_bytes())
    }

    pub(crate) fn take_commands(&self) -> io::Result<String> {
        let mut st = self.state();
        let session = &mut st.session;
        self.inner
            .term
            .with_link(|link, now| session.flush(link, now))
    }

    pub(crate) fn watch(&self, output: &[u8]) {
        self.state().session.watch(output);
    }

    /// Asks the terminal to answer with the next frame: [`Event::Pong`]
    /// says how long it took to take the frame written before it.
    pub fn ping(&self) {
        let now = self.inner.term.now();
        self.state().session.ping(now);
    }

    /// Detaches every surface the session has sent (SPEC §5.5), for a
    /// program that leaves them on the screen when it quits. It leaves with
    /// the next frame, or [`Session::flush`].
    pub fn detach_all(&self) {
        self.state().session.detach_all();
    }

    /// Ends the session on the program's way out: deletes every surface it
    /// sent, withdraws a late query no answer came to, gives the terminal
    /// its mode back and stops reading it. [`restore`](crate::restore) calls
    /// it; a program that leaves its surfaces on screen detaches them first.
    pub fn close(&self) -> io::Result<()> {
        let out = {
            let mut st = self.state();
            let session = &mut st.session;
            self.inner
                .term
                .with_link(|link, now| session.close(link, now))?
        };
        let r = self.inner.term.write(out.as_bytes());
        self.inner
            .raw
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(h) = self
            .inner
            .signals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            h.close();
        }
        self.inner.term.close();
        r
    }
}
