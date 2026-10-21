//! The process's terminal (SDK.md §4.1): for a program that is not a
//! full-screen framework's, such as a command that prints a card and exits,
//! one that asks a question, or a chart that streams.
//!
//! ```no_run
//! use hotty::{Placement, Terminal};
//!
//! # fn main() -> std::io::Result<()> {
//! let Ok(t) = Terminal::open("hello") else {
//!     println!("Hello."); // no terminal: a pipe, a cron job
//!     return Ok(());
//! };
//! if t.detect() {
//!     t.line_start(); // surfaces are placed at the cursor
//!     t.print("hello", "<p>Hello, <b>HTML</b> in the terminal.</p>", &Placement::new(40, 1))?;
//!     t.fence().ok(); // the host's replies, before the shell reads the terminal
//! } else {
//!     println!("Hello."); // a terminal that is not a host, or not of this version
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Everything the terminal sends is read once, by the Terminal, on a thread
//! of its own: keys, the mouse, pastes, and hotty replies and events. What a
//! call waits for goes to that call; the rest goes to the input stream
//! ([`Terminal::next_input`]), in order. Keys come as the bytes the
//! terminal sent: decoding them is the program's, or its key library's
//! ([`decode_keys`](crate::decode_keys)).
//!
//! A command that exits hands the terminal to whatever runs next, usually a
//! shell, which reads what the terminal sends as typing. So before it
//! exits, a program reads every reply its commands caused
//! ([`Terminal::fence`]), and leaves no surface that still reports events:
//! it sends documents detached ([`Terminal::print`] does), or detaches them
//! ([`Terminal::detach_all`]). Closing ([`Terminal::close`], or dropping
//! the Terminal) gives the terminal back as it was.

mod tty;

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::commands::{Placement, doc, place};
use crate::link::{Answer, Input, Known, Link, NoAnswer, Ticket};
use crate::messages::{Caps, Reply};

pub use tty::{RawMode, Tty};

/// What a [`Terminal`]'s calls fail with.
#[derive(Debug)]
pub enum Error {
    /// Writing to the terminal failed.
    Io(io::Error),
    /// The terminal did not answer in time, or its input ended.
    NoAnswer(NoAnswer),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "hotty: {e}"),
            Error::NoAnswer(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::NoAnswer(e) => Some(e),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<NoAnswer> for Error {
    fn from(e: NoAnswer) -> Error {
        Error::NoAnswer(e)
    }
}

impl From<Error> for io::Error {
    fn from(e: Error) -> io::Error {
        match e {
            Error::Io(e) => e,
            Error::NoAnswer(e) => io::Error::new(io::ErrorKind::TimedOut, e),
        }
    }
}

type Source = Box<dyn FnMut(&mut [u8]) -> io::Result<usize> + Send>;

struct State {
    link: Link,
    seen: u64,
    source: Option<Source>,
}

struct Raw {
    tty: Option<Arc<Tty>>,
    depth: usize,
    mode: Option<RawMode>,
}

struct Shared {
    state: Mutex<State>,
    cond: Condvar,
    out: Mutex<Box<dyn Write + Send>>,
    raw: Mutex<Raw>,
    epoch: Instant,
}

impl Shared {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Writes what the link queued, in the order it was queued, without
    // holding the state while the write blocks.
    fn flush(&self, mut st: MutexGuard<'_, State>, extra: &[u8]) -> io::Result<()> {
        let mut bytes = st.link.take_output();
        bytes.extend_from_slice(extra);
        if bytes.is_empty() {
            return Ok(());
        }
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        drop(st);
        out.write_all(&bytes)?;
        out.flush()
    }

    // Raw mode while a call waits, detection runs, a late answer may come,
    // or the program holds it.
    fn sync_raw(&self, waiting: bool) {
        let mut raw = self.raw.lock().unwrap_or_else(|e| e.into_inner());
        let want = raw.depth > 0 || waiting;
        match (&raw.tty, want, raw.mode.is_some()) {
            (Some(tty), true, false) => raw.mode = tty.raw().ok(),
            (_, false, true) => raw.mode = None,
            _ => {}
        }
    }
}

/// Builds a [`Terminal`]: [`Terminal::builder`].
#[derive(Debug, Default)]
pub struct Builder {
    name: String,
    known: Option<Known>,
    late: bool,
    size: Option<(u16, u16)>,
}

impl Builder {
    /// What [`Terminal::detect`] answers without asking: the terminal is
    /// known to be a host, or not, and with what capabilities.
    #[must_use]
    pub fn known(mut self, known: Known) -> Builder {
        self.known = Some(known);
        self
    }

    /// Asks for a late answer (SPEC §4): if one comes, it goes to the input
    /// stream as [`Input::Host`], and [`Terminal::native`] answers true from
    /// then on. Until it comes, the terminal stays in raw mode, or the
    /// answer would be echoed, and [`Terminal::close`] withdraws the query.
    #[must_use]
    pub fn late(mut self, late: bool) -> Builder {
        self.late = late;
        self
    }

    /// The terminal's size in cells, for a terminal made from streams.
    #[must_use]
    pub fn size(mut self, cols: u16, rows: u16) -> Builder {
        self.size = Some((cols, rows));
        self
    }

    /// Opens the process's terminal, `/dev/tty`, whatever its standard
    /// streams are: `x=$(mytool ask)` draws on the terminal and prints to
    /// the pipe. The name and the process id name its surfaces. Fails when
    /// the process has no terminal (a pipe, a cron job): the program writes
    /// plain data.
    pub fn open(self) -> io::Result<Terminal> {
        let tty = Arc::new(Tty::open()?);
        let name = format!("{}-{}", self.name, std::process::id());
        let (wake_r, wake_w) = rustix::pipe::pipe()?;
        let reader = tty.clone();
        let source: Source = Box::new(move |buf| reader.read_or_wake(buf, &wake_r));
        let writer = tty.clone();
        let t = Terminal::make(
            &name,
            self.known,
            self.late,
            source,
            Box::new(TtyWriter(writer)),
            Some(tty),
            self.size,
        );
        *t.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake_w);
        Ok(t)
    }

    /// A terminal from its two streams, for a test or an environment with
    /// no file. Its input is read on a thread of its own, once something
    /// asks.
    pub fn io(
        self,
        input: impl Read + Send + 'static,
        output: impl Write + Send + 'static,
    ) -> Terminal {
        let mut input = input;
        Terminal::make(
            &self.name,
            self.known,
            self.late,
            Box::new(move |buf| input.read(buf)),
            Box::new(output),
            None,
            self.size,
        )
    }
}

struct TtyWriter(Arc<Tty>);

impl Write for TtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One process's terminal (SDK.md §4.1). Calls take `&self`, so a thread
/// that reads the input stream and one that waits for a reply may run at
/// once.
pub struct Terminal {
    shared: Arc<Shared>,
    tty: Option<Arc<Tty>>,
    size: Option<(u16, u16)>,
    wake: Mutex<Option<rustix::fd::OwnedFd>>,
    reader: Mutex<Option<thread::JoinHandle<()>>>,
}

impl fmt::Debug for Terminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terminal")
            .field("tty", &self.tty.is_some())
            .finish_non_exhaustive()
    }
}

impl Terminal {
    /// Opens the process's terminal: [`Builder::open`] with the defaults.
    pub fn open(name: &str) -> io::Result<Terminal> {
        Terminal::builder(name).open()
    }

    /// A builder for a terminal whose surfaces are named after `name`.
    pub fn builder(name: &str) -> Builder {
        Builder {
            name: name.to_string(),
            ..Builder::default()
        }
    }

    fn make(
        name: &str,
        known: Option<Known>,
        late: bool,
        source: Source,
        out: Box<dyn Write + Send>,
        tty: Option<Arc<Tty>>,
        size: Option<(u16, u16)>,
    ) -> Terminal {
        let mut link = Link::new(name).late(late);
        if let Some(k) = known {
            link = link.known(k);
        }
        Terminal {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    link,
                    seen: 0,
                    source: Some(source),
                }),
                cond: Condvar::new(),
                out: Mutex::new(out),
                raw: Mutex::new(Raw {
                    tty: tty.clone(),
                    depth: 0,
                    mode: None,
                }),
                epoch: Instant::now(),
            }),
            tty,
            size,
            wake: Mutex::new(None),
            reader: Mutex::new(None),
        }
    }

    // Starts reading the input, once.
    fn read(&self) {
        let mut st = self.shared.lock();
        let Some(mut source) = st.source.take() else {
            return;
        };
        drop(st);
        let shared = self.shared.clone();
        let handle = thread::Builder::new()
            .name("hotty-input".into())
            .spawn(move || {
                let mut buf = vec![0u8; 8192];
                loop {
                    let n = source(&mut buf);
                    let now = shared.now();
                    let mut st = shared.lock();
                    match n {
                        Ok(n) if n > 0 => st.link.bytes(&buf[..n], now),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {
                            // Woken by close: stop reading, as it was.
                            st.seen += 1;
                            drop(st);
                            shared.cond.notify_all();
                            return;
                        }
                        _ => st.link.end(now),
                    }
                    st.seen += 1;
                    let (waiting, ended) = (st.link.waiting(), st.link.ended());
                    drop(st);
                    shared.cond.notify_all();
                    shared.sync_raw(waiting);
                    if ended {
                        return;
                    }
                }
            });
        if let Ok(h) = handle {
            *self.reader.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
        }
    }

    // Starts a call, writes what it queued, and waits for its answer, the
    // terminal raw meanwhile.
    fn call(&self, start: impl FnOnce(&mut Link, u64) -> Ticket) -> Result<Answer, Error> {
        self.read();
        self.shared.sync_raw(true);
        let mut st = self.shared.lock();
        let ticket = start(&mut st.link, self.shared.now());
        self.shared.flush(st, b"")?;
        let mut st = self.shared.lock();
        let answer = loop {
            if let Some(a) = st.link.answer(ticket) {
                break a;
            }
            st = self.sleep(st, None);
        };
        let waiting = st.link.waiting();
        drop(st);
        self.shared.sync_raw(waiting);
        Ok(answer?)
    }

    // Waits for the reader or the next deadline, at most until `until`,
    // and lets the time pass.
    fn sleep<'a>(&'a self, st: MutexGuard<'a, State>, until: Option<u64>) -> MutexGuard<'a, State> {
        let now = self.shared.now();
        let at = match (st.link.deadline(), until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut st = match at {
            Some(at) => {
                let d = Duration::from_millis(at.saturating_sub(now).max(1));
                self.shared
                    .cond
                    .wait_timeout(st, d)
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            }
            None => self.shared.cond.wait(st).unwrap_or_else(|e| e.into_inner()),
        };
        st.link.tick(self.shared.now());
        st
    }

    /// Asks the terminal whether it is a hotty host that speaks this SDK's
    /// version (SPEC §4), once: later calls answer what the first found, or
    /// true once a late answer came. It returns when the detection is done,
    /// so that nothing of it is left for whatever reads the terminal next.
    /// A terminal that answers the query with an error (`EVERSION`) or with
    /// capabilities of another version, or answers neither the query nor
    /// DA1 within 1.5 s, is not a host.
    pub fn detect(&self) -> bool {
        self.detect_until(Link::detected)
    }

    /// As [`Terminal::detect`], but returns as soon as it is decided: for a
    /// program that must not wait. Whatever of the detection is still to
    /// come, the Terminal reads and swallows.
    pub fn detect_decided(&self) -> bool {
        self.detect_until(Link::decided)
    }

    fn detect_until(&self, ready: fn(&Link) -> bool) -> bool {
        self.read();
        let mut st = self.shared.lock();
        if !st.link.started() {
            drop(st);
            self.shared.sync_raw(true);
            st = self.shared.lock();
            st.link.detect(self.shared.now(), false);
            if self.shared.flush(st, b"").is_err() {
                let mut st = self.shared.lock();
                st.link.end(self.shared.now());
            }
            st = self.shared.lock();
        }
        while !ready(&st.link) {
            st = self.sleep(st, None);
        }
        let (native, waiting) = (st.link.native(), st.link.waiting());
        drop(st);
        self.shared.sync_raw(waiting);
        native
    }

    /// What detection found, without waiting: true for a hotty host. False
    /// until it is decided; true from a late answer on.
    pub fn native(&self) -> bool {
        self.shared.lock().link.native()
    }

    /// The host's capabilities, when native, kept current by the `caps`
    /// events it has sent since ([`Caps::merge`]).
    pub fn caps(&self) -> Option<Caps> {
        self.shared.lock().link.caps().cloned()
    }

    /// Sends one command numbered with `n` and waits for its reply, 3 s at
    /// most. An error reply is a reply ([`Reply::err`]), not an [`Error`].
    ///
    /// ```no_run
    /// # let t = hotty::Terminal::open("x")?;
    /// let r = t.request(hotty::place("x-card", &hotty::Placement::new(40, 0)))?;
    /// let rows = r.rows; // the rows the host chose
    /// # Ok::<(), hotty::term::Error>(())
    /// ```
    pub fn request(&self, command: impl Into<crate::Cmd>) -> Result<Reply, Error> {
        let command = command.into();
        match self.call(|l, now| l.request(command, now))? {
            Answer::Reply(r) => Ok(r),
            other => unreachable!("a request answered {other:?}"),
        }
    }

    /// Waits until the terminal has taken everything written before it (it
    /// sends DA1, which terminals answer in order), 1 s at most, and returns
    /// the replies nothing read: the errors of commands sent with `q=1`,
    /// mostly. A program calls it before it exits, after the last command
    /// that may be answered, so that no reply is left for the shell to read
    /// as typing. No answer in time is [`Error::NoAnswer`], which carries
    /// the replies.
    pub fn fence(&self) -> Result<Vec<Reply>, Error> {
        match self.call(Link::fence)? {
            Answer::Fence(r) => Ok(r),
            other => unreachable!("a fence answered {other:?}"),
        }
    }

    /// Writes DA1 and returns how long the terminal took to answer it.
    pub fn ping(&self) -> Result<Duration, Error> {
        match self.call(Link::ping)? {
            Answer::Da1 { took } => Ok(Duration::from_millis(took)),
            other => unreachable!("a ping answered {other:?}"),
        }
    }

    /// Asks where the cursor is (CPR), and returns its column and row from
    /// 0, 1 s at most: through the Terminal, which reads the answer out of
    /// the input, where asking another way would race it.
    pub fn cursor_position(&self) -> Result<(u32, u32), Error> {
        match self.call(Link::cursor_position)? {
            Answer::Cursor { row, col } => Ok((col, row)),
            other => unreachable!("the cursor's position answered {other:?}"),
        }
    }

    /// Moves the cursor to the start of a line, where a surface placed at
    /// the cursor belongs: it asks where the cursor is (CPR), and writes CR
    /// LF if it is not at column 0. A terminal that does not answer within
    /// 1 s is taken to be at one.
    pub fn line_start(&self) {
        if let Ok((col, _)) = self.cursor_position()
            && col > 0
        {
            let _ = self.write(b"\r\n");
        }
    }

    /// Shows a document at the cursor, to stay in the scrollback among the
    /// program's output: the surface named `name`, made this program's own
    /// ([`Terminal::surface`]), gets `html`, detached, and is placed, in one
    /// write. The host moves the cursor below it, unless `keep_cursor`.
    /// Returns the surface's full name, for deltas. Errors the host replies
    /// come back on the input: read them with [`Terminal::fence`].
    pub fn print(&self, name: &str, html: &str, placement: &Placement) -> io::Result<String> {
        let s = self.surface(name);
        self.send(format_args!(
            "{}{}",
            doc(&s, html).detached(),
            place(&s, placement)
        ))?;
        Ok(s)
    }

    /// Writes commands to the terminal in one write, so that nothing the
    /// program prints lands between them.
    pub fn send(&self, commands: impl fmt::Display) -> io::Result<()> {
        let mut st = self.shared.lock();
        st.link.send(commands);
        self.shared.flush(st, b"")
    }

    /// Writes bytes that are not hotty's to the terminal, in order with its
    /// commands.
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let st = self.shared.lock();
        self.shared.flush(st, bytes)
    }

    /// A surface name of this program's own: the terminal's name (the
    /// program's and its process id), a dash, and `name`, made a valid
    /// surface name, so that two runs never share a surface. Every name it
    /// returns is remembered ([`Terminal::surfaces`]).
    pub fn surface(&self, name: &str) -> String {
        self.shared.lock().link.surface(name)
    }

    /// The names [`Terminal::surface`] has returned: what this program may
    /// have left on the terminal.
    pub fn surfaces(&self) -> Vec<String> {
        self.shared.lock().link.surfaces().to_vec()
    }

    /// Detaches every surface [`Terminal::surface`] has named (SPEC §5.5),
    /// so that none reports to whatever reads the terminal after this
    /// program.
    pub fn detach_all(&self) -> io::Result<()> {
        let mut st = self.shared.lock();
        st.link.detach_all();
        self.shared.flush(st, b"")
    }

    /// The next input no call waits for: a hotty event or a reply nothing
    /// read, the bytes of keys, the mouse and pastes, or a late answer.
    /// Waits for one, `timeout` at most (`None`: until one comes); `None`
    /// when the time is up, or the input has ended.
    pub fn next_input(&self, timeout: Option<Duration>) -> Option<Input> {
        self.read();
        let until = timeout.map(|t| self.shared.now() + t.as_millis() as u64);
        let mut st = self.shared.lock();
        loop {
            if let Some(i) = st.link.poll_input() {
                return Some(i);
            }
            let now = self.shared.now();
            if st.link.ended() || st.link.closed() || until.is_some_and(|u| now >= u) {
                return None;
            }
            st = self.sleep(st, until);
        }
    }

    /// Puts the terminal in raw mode (no echo, no line editing, keys as
    /// they are typed) until the guard is dropped. Calls that wait for the
    /// terminal do this for themselves; a program that reads keys holds one.
    /// In raw mode `\n` does not return the carriage: write `\r\n`.
    pub fn raw(&self) -> RawGuard {
        self.shared
            .raw
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .depth += 1;
        self.shared.sync_raw(true);
        RawGuard(self.shared.clone())
    }

    /// Wakes whatever waits in [`Terminal::wait`] or
    /// [`Terminal::next_input`], as input arriving would: for an adapter
    /// that learns of something else, such as a resize.
    pub fn wake(&self) {
        self.shared.lock().seen += 1;
        self.shared.cond.notify_all();
    }

    /// Whether the input has ended, or the terminal is closed.
    pub fn ended(&self) -> bool {
        let st = self.shared.lock();
        st.link.ended() || st.link.closed()
    }

    /// The terminal's size in cells, or 80 × 24 when it cannot be told.
    pub fn size(&self) -> (u16, u16) {
        self.tty
            .as_ref()
            .and_then(|t| t.size())
            .or(self.size)
            .filter(|&(c, r)| c > 0 && r > 0)
            .unwrap_or((80, 24))
    }

    /// Asks the terminal whether it shows kitty graphics, so that a program
    /// on a terminal that is not a host can suggest a polyfill (SPEC §14).
    /// Ask after [`Terminal::detect`], and only when it is false.
    pub fn kitty_graphics(&self) -> bool {
        matches!(self.call(Link::kitty_graphics), Ok(Answer::Graphics(true)))
    }

    /// Runs `f` on the terminal's [`Link`], and writes what it queued: for an
    /// adapter that stands on the Terminal, such as a framework's session.
    pub fn with_link<R>(&self, f: impl FnOnce(&mut Link, u64) -> R) -> io::Result<R> {
        self.read();
        let mut st = self.shared.lock();
        let r = f(&mut st.link, self.shared.now());
        let waiting = st.link.waiting();
        self.shared.flush(st, b"")?;
        self.shared.sync_raw(waiting);
        Ok(r)
    }

    /// Waits until the terminal reads something, or the time is up, and
    /// lets the time pass: for an adapter that polls the [`Link`] itself.
    /// Returns whether something arrived.
    pub fn wait(&self, timeout: Duration) -> bool {
        self.read();
        let until = self.shared.now() + timeout.as_millis() as u64;
        let mut st = self.shared.lock();
        let seen = st.seen;
        while st.seen == seen && !st.link.has_input() && self.shared.now() < until {
            if st.link.ended() || st.link.closed() {
                return false;
            }
            st = self.sleep(st, Some(until));
        }
        st.seen != seen || st.link.has_input()
    }

    /// The time the terminal's [`Link`] is handed, in milliseconds.
    pub fn now(&self) -> u64 {
        self.shared.now()
    }

    /// Gives the terminal back as it was (its mode) and stops reading it;
    /// the input stream ends. It sends nothing but the withdrawal of a late
    /// query no answer came to: what is left on the screen, and the replies
    /// not read, are the program's to settle first (SDK.md §2.6).
    pub fn close(&self) {
        let mut st = self.shared.lock();
        if st.link.closed() {
            return;
        }
        st.link.close(self.shared.now());
        let _ = self.shared.flush(st, b"");
        self.shared.cond.notify_all();
        {
            let mut raw = self.shared.raw.lock().unwrap_or_else(|e| e.into_inner());
            raw.depth = 0;
            raw.mode = None;
        }
        if let Some(w) = self.wake.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = rustix::io::write(&w, b"x");
            if let Some(h) = self.reader.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}

/// Raw mode, held until dropped: [`Terminal::raw`].
#[must_use = "raw mode ends when the guard is dropped"]
pub struct RawGuard(Arc<Shared>);

impl Drop for RawGuard {
    fn drop(&mut self) {
        let s = &self.0;
        {
            let mut raw = s.raw.lock().unwrap_or_else(|e| e.into_inner());
            raw.depth = raw.depth.saturating_sub(1);
        }
        let waiting = s.lock().link.waiting();
        s.sync_raw(waiting);
    }
}

impl fmt::Debug for RawGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RawGuard")
    }
}
