//! The program's link to a host, sans I/O (SDK.md §4.1, §4.2).
//!
//! A [`Link`] is handed what the terminal sends, as a byte stream
//! ([`Link::bytes`]) or as complete sequences an environment's input parser
//! cut out ([`Link::osc`], [`Link::da1`]), and the time, in milliseconds
//! from any origin. It routes what arrives: what a call waits for goes to
//! that call (detection's answers, a fence's DA1, a numbered reply), and
//! everything else to one stream of [`Input`]s, in the order it arrived. What
//! it writes waits in [`Link::take_output`] for the environment to write.
//!
//! [`Terminal`](crate::Terminal) is a Link on the process's terminal, and a
//! [`Session`](crate::Session) stands on one. An environment that does its
//! own I/O drives one itself:
//!
//! ```
//! use hotty::link::{Input, Link};
//!
//! let mut link = Link::new("tool-42");
//! link.detect(0, false);
//! let query = link.take_output(); // write it to the terminal
//! # assert!(query.ends_with(b"\x1b[c"));
//! // … then hand it what the terminal sends, and the time:
//! link.bytes(b"\x1b[?62;22c", 20); // a DA1 with no reply before it
//! link.tick(200);
//! assert!(link.decided() && !link.native()); // not a host: draw in cells
//! ```

use std::collections::VecDeque;
use std::fmt;

use crate::commands::{Cmd, detach, withdraw_late};
use crate::decoder::{Decoder, Fed};
use crate::detector::{DETECT_AFTER_DA1, DETECT_TIMEOUT, Detector, State};
use crate::messages::{Caps, Message, Reply};
use crate::names::surface_name;
use crate::scanner::{Scanner, Segment};

/// How long a request waits for its reply, in milliseconds: a terminal that
/// is not a host never replies.
pub const REPLY_TIMEOUT: u64 = 3000;
/// How long a fence, a ping or the cursor's position waits, in
/// milliseconds: every terminal answers at once, or never.
pub const FENCE_TIMEOUT: u64 = 1000;
/// The query's number: a reply that does not echo it answers someone else's.
pub const QUERY_N: u64 = 1;
/// How long a lone ESC at the end of a read waits for what may follow it
/// before it goes out as the Escape key, in milliseconds: a hotty sequence
/// cut right after its ESC still arrives whole.
pub const ESCAPE_DELAY: u64 = 25;

const DA1: &str = "\x1b[c";
// A 1×1 RGB image with id 31, which a terminal that has kitty graphics
// answers with ESC _Gi=31;OK ESC \ and draws nothing, then DA1, which
// every terminal answers after it.
const GRAPHICS_QUERY: &str = "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";

/// What a terminal is known to be without asking it, such as one in a page
/// that always has the hotty addon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known {
    /// Whether it is a host.
    pub native: bool,
    /// Its capabilities, when it is.
    pub caps: Option<Caps>,
}

/// What arrives that no call waits for, in the order it arrived.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // passed now and then, not stored in bulk
pub enum Input {
    /// A hotty message: an event, or a reply nothing waits for.
    Message(Message),
    /// Bytes of a byte stream that are not hotty's: keys, the mouse,
    /// pastes, other answers, unchanged.
    Data(Vec<u8>),
    /// A late answer to the query (SPEC §4): the terminal is a host from
    /// now on, with these capabilities.
    Host(Option<Caps>),
}

/// A call waited for the terminal in vain: it did not answer in time, or
/// its input ended.
#[derive(Clone, Debug, PartialEq)]
pub struct NoAnswer {
    /// What was waited for.
    pub what: String,
    /// A fence's replies that came before it gave up.
    pub replies: Vec<Reply>,
}

impl fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hotty: the terminal did not answer {}", self.what)
    }
}

impl std::error::Error for NoAnswer {}

/// A call that waits for the terminal: [`Link::request`], [`Link::fence`],
/// …. [`Link::answer`] takes its answer once it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

/// What a waiting call got.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // passed now and then, not stored in bulk
pub enum Answer {
    /// A request's reply, ok or an error.
    Reply(Reply),
    /// A fence's DA1 came: the replies nothing read before it.
    Fence(Vec<Reply>),
    /// A DA1 came, `took` milliseconds after it was asked for.
    Da1 {
        /// Milliseconds from the call to the answer.
        took: u64,
    },
    /// The cursor's position, from 0.
    Cursor {
        /// The row.
        row: u32,
        /// The column.
        col: u32,
    },
    /// Whether the terminal shows kitty graphics.
    Graphics(bool),
}

#[derive(Debug)]
enum Kind {
    Reply(u64),
    Fence(Vec<Reply>),
    Da1,
    Cursor,
    Graphics {
        answer: Option<bool>,
        da1: bool,
        until: Option<u64>,
    },
}

#[derive(Debug)]
struct Wait {
    ticket: Ticket,
    kind: Kind,
    what: &'static str,
    since: u64,
    deadline: u64,
    carry: Vec<u8>,
}

/// The program's link to a host (SDK.md §4.1, §4.2), sans I/O.
#[derive(Debug)]
pub struct Link {
    name: String,
    known: Option<Known>,
    late: bool,
    escape_delay: u64,
    decoder: Decoder,
    scanner: Scanner,
    esc_at: Option<u64>,
    out: Vec<u8>,
    queue: VecDeque<Input>,
    waits: Vec<Wait>,
    answers: Vec<(Ticket, Result<Answer, NoAnswer>)>,
    next_ticket: u64,
    names: Vec<String>,
    next_n: u64,
    ended: bool,
    closed: bool,
    detector: Option<Detector>,
    started: bool,
    native: bool,
    caps: Option<Caps>,
    withdrawn: bool,
}

impl Link {
    /// A link whose surface names ([`Link::surface`]) start with `name`: the
    /// program's name and a number unique on the terminal, such as its
    /// process id, or what the environment provides. Made a valid name.
    pub fn new(name: &str) -> Link {
        Link {
            name: surface_name(name),
            known: None,
            late: false,
            escape_delay: ESCAPE_DELAY,
            decoder: Decoder::new(),
            scanner: Scanner::new().da1(true),
            esc_at: None,
            out: Vec::new(),
            queue: VecDeque::new(),
            waits: Vec::new(),
            answers: Vec::new(),
            next_ticket: 0,
            names: Vec::new(),
            next_n: QUERY_N,
            ended: false,
            closed: false,
            detector: None,
            started: false,
            native: false,
            caps: None,
            withdrawn: false,
        }
    }

    /// What detection answers without asking: the terminal is known to be a
    /// host, or not, and with what capabilities.
    #[must_use]
    pub fn known(mut self, known: Known) -> Link {
        self.known = Some(known);
        self
    }

    /// Ask for a late answer (SPEC §4), for a program that can start using
    /// hotty after it fell back: what passes hotty through with no host
    /// yet, such as a multiplexer's pane with no terminal attached, may
    /// answer once a terminal that is a host attaches. The answer comes to
    /// the input stream as [`Input::Host`], and the link is native from
    /// then on. Until it comes, [`Link::close`] withdraws the query.
    #[must_use]
    pub fn late(mut self, late: bool) -> Link {
        self.late = late;
        self
    }

    /// How long a lone ESC at the end of a read waits before it is the
    /// Escape key: [`ESCAPE_DELAY`] by default, 0 for at once.
    #[must_use]
    pub fn escape_delay(mut self, ms: u64) -> Link {
        self.escape_delay = ms;
        self
    }

    /// The prefix of the surface names [`Link::surface`] makes.
    pub fn name(&self) -> &str {
        &self.name
    }

    // --- output ---------------------------------------------------------

    /// Queues commands for the terminal, to go out with the next
    /// [`Link::take_output`]. Nothing after [`Link::close`].
    pub fn send(&mut self, commands: impl fmt::Display) {
        if !self.closed {
            use fmt::Write as _;
            let mut s = String::new();
            let _ = write!(s, "{commands}");
            self.out.extend_from_slice(s.as_bytes());
        }
    }

    /// What is queued for the terminal, to write in one write.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    // --- input ----------------------------------------------------------

    /// Takes the next bytes of the terminal's input, however its reads
    /// split it: hotty's sequences and DA1 answers are cut out of them (a
    /// [`Scanner`]). A lone ESC at the end of a read is the Escape key, once
    /// nothing follows it within the escape delay ([`Link::tick`]).
    pub fn bytes(&mut self, chunk: &[u8], now: u64) {
        if self.closed {
            return;
        }
        self.tick(now);
        self.esc_at = None;
        let segs = self.scanner.feed(chunk);
        self.segments(segs, now);
        if self.scanner.holding() == b"\x1b" {
            if self.escape_delay == 0 {
                let segs = self.scanner.flush();
                self.segments(segs, now);
            } else {
                self.esc_at = Some(now + self.escape_delay);
            }
        }
    }

    fn segments(&mut self, segs: Vec<Segment>, now: u64) {
        for s in segs {
            match s {
                Segment::Osc(seq) => {
                    self.osc_at(&seq, now);
                }
                Segment::Da1(seq) => {
                    if !self.da1_at(now) {
                        self.data(seq, now);
                    }
                }
                Segment::Pass(b) => self.data(b, now),
            }
        }
    }

    /// Takes one complete OSC sequence, with or without its terminator, as
    /// an environment's input parser hands it over. Returns whether it was
    /// hotty's: a sequence that is not goes back to the environment.
    pub fn osc(&mut self, seq: &[u8], now: u64) -> bool {
        if self.closed {
            return false;
        }
        self.tick(now);
        self.osc_at(seq, now)
    }

    fn osc_at(&mut self, seq: &[u8], now: u64) -> bool {
        match self.decoder.feed(seq) {
            Fed::NotHotty => false,
            Fed::Complete(m) => {
                self.message(m, now);
                true
            }
            Fed::Partial | Fed::Invalid => true,
        }
    }

    /// A DA1 answer arrived, as an environment's input parser hands it over.
    /// Returns whether it was this link's: detection's, a fence's or a
    /// ping's, for the environment to swallow. One it does not take answers
    /// someone else's question.
    pub fn da1(&mut self, now: u64) -> bool {
        if self.closed {
            return false;
        }
        self.tick(now);
        self.da1_at(now)
    }

    fn da1_at(&mut self, now: u64) -> bool {
        if let Some(d) = self.detector.as_mut().filter(|d| !d.done()) {
            let took = d.da1(now);
            self.on_detect();
            if took {
                return true;
            }
        }
        for i in 0..self.waits.len() {
            let w = &mut self.waits[i];
            let answer = match &mut w.kind {
                Kind::Fence(replies) => Answer::Fence(std::mem::take(replies)),
                Kind::Da1 => Answer::Da1 {
                    took: now.saturating_sub(w.since),
                },
                Kind::Graphics { answer, da1, until } if !*da1 => {
                    *da1 = true;
                    match answer {
                        // The DA1 behind the answer: done.
                        Some(ok) => Answer::Graphics(*ok),
                        // A DA1 first may answer an earlier question: the
                        // answer may still come, for a moment.
                        None => {
                            *until = Some(now + DETECT_AFTER_DA1);
                            return true;
                        }
                    }
                }
                _ => continue,
            };
            self.finish(i, Ok(answer));
            return true;
        }
        false
    }

    /// The input ended: calls that wait give up, and what the scanner held
    /// goes to the input stream.
    pub fn end(&mut self, now: u64) {
        if self.ended {
            return;
        }
        self.esc_at = None;
        let held = self.scanner.flush();
        self.segments(held, now);
        self.ended = true;
        if let Some(d) = self.detector.as_mut().filter(|d| !d.done()) {
            d.end(now);
            self.on_detect();
        }
        self.fail_all();
    }

    /// Whether the input has ended.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// The time is now: a lone ESC whose delay has passed goes out as the
    /// Escape key, detection moves on, and calls whose time is up give up.
    pub fn tick(&mut self, now: u64) {
        if self.esc_at.is_some_and(|at| now >= at) {
            self.esc_at = None;
            if self.scanner.holding() == b"\x1b" {
                let segs = self.scanner.flush();
                self.segments(segs, now);
            }
        }
        if let Some(d) = self.detector.as_mut().filter(|d| !d.done()) {
            d.tick(now);
            self.on_detect();
        }
        let mut i = 0;
        while i < self.waits.len() {
            let w = &self.waits[i];
            if let Kind::Graphics {
                answer,
                until: Some(until),
                ..
            } = w.kind
                && now >= until
            {
                self.finish(i, Ok(Answer::Graphics(answer.unwrap_or(false))));
                continue;
            }
            if now >= w.deadline {
                let w = self.waits.remove(i);
                let result = match w.kind {
                    Kind::Graphics { answer, .. } => Ok(Answer::Graphics(answer.unwrap_or(false))),
                    Kind::Fence(replies) => Err(NoAnswer {
                        what: w.what.to_string(),
                        replies,
                    }),
                    _ => Err(NoAnswer {
                        what: w.what.to_string(),
                        replies: Vec::new(),
                    }),
                };
                self.answers.push((w.ticket, result));
                continue;
            }
            i += 1;
        }
    }

    /// When to call [`Link::tick`] next: the escape delay's end, detection's
    /// deadline, or the first call's time running out; `None` when nothing
    /// waits.
    pub fn deadline(&self) -> Option<u64> {
        let mut d = self.esc_at;
        let mut at = |t: Option<u64>| {
            if let Some(t) = t {
                d = Some(d.map_or(t, |d| d.min(t)));
            }
        };
        at(self.detector.as_ref().and_then(Detector::deadline));
        for w in &self.waits {
            at(Some(w.deadline));
            if let Kind::Graphics { until, .. } = w.kind {
                at(until);
            }
        }
        d
    }

    /// Whether a call waits for the terminal, or detection is under way, or
    /// a late answer may still come: while it does, a terminal stays in raw
    /// mode, or what the terminal answers would be echoed.
    pub fn waiting(&self) -> bool {
        !self.waits.is_empty()
            || self.detector.as_ref().is_some_and(|d| !d.done())
            || self.late_pending()
    }

    fn late_pending(&self) -> bool {
        self.detector
            .as_ref()
            .is_some_and(|d| d.is_late() && d.state() != State::Native)
            && !self.withdrawn
            && !self.closed
    }

    fn message(&mut self, m: Message, now: u64) {
        if let Some(r) = m.reply()
            && let Some(d) = self.detector.as_mut()
        {
            // Text: no host answered, or the one that did is not one this
            // SDK can use, and a reply to the query that makes it one is a
            // late answer.
            let fell_back = d.state() == State::Text;
            // A withdrawn query's late answer reaches nothing.
            if fell_back && self.withdrawn && r.re == "q" && r.n == Some(d.n()) {
                return;
            }
            // Detection's, ok or an error, whenever it arrives: swallowed.
            if d.reply(&r, now) {
                if fell_back && d.state() == State::Native {
                    let caps = d.caps().cloned();
                    self.native = true;
                    self.caps.clone_from(&caps);
                    self.queue.push_back(Input::Host(caps));
                } else {
                    self.on_detect();
                }
                return;
            }
        }
        // A caps event folds into the capabilities (SPEC §4); it still
        // reaches the program, which may act on the change.
        if let Some(ev) = m.event().filter(|e| e.kind == crate::kind::CAPS)
            && let (Some(ch), Some(caps)) = (ev.changed(), self.caps.as_ref())
        {
            self.caps = Some(caps.merge(ch));
        }
        if let Some(r) = m.reply() {
            for i in 0..self.waits.len() {
                match &mut self.waits[i].kind {
                    Kind::Reply(n) if r.n == Some(*n) => {
                        self.finish(i, Ok(Answer::Reply(r)));
                        return;
                    }
                    Kind::Fence(replies) if r.n.is_none() => {
                        replies.push(r);
                        return;
                    }
                    _ => {}
                }
            }
        }
        self.queue.push_back(Input::Message(m));
    }

    fn data(&mut self, mut b: Vec<u8>, now: u64) {
        let _ = now;
        for i in 0..self.waits.len() {
            if b.is_empty() {
                return;
            }
            let w = &mut self.waits[i];
            let pattern = match w.kind {
                Kind::Cursor => Pattern::Cursor,
                Kind::Graphics { answer: None, .. } => Pattern::Graphics,
                _ => continue,
            };
            let mut all = std::mem::take(&mut w.carry);
            all.extend_from_slice(&b);
            if let Some((start, end, value)) = pattern.find(&all) {
                let mut rest = all[..start].to_vec();
                rest.extend_from_slice(&all[end..]);
                match (&mut w.kind, value) {
                    (Kind::Cursor, Found::Cursor(row, col)) => {
                        self.finish(i, Ok(Answer::Cursor { row, col }));
                        b = rest;
                        break;
                    }
                    (Kind::Graphics { answer, da1, until }, Found::Graphics(ok)) => {
                        *answer = Some(ok);
                        if *da1 {
                            self.finish(i, Ok(Answer::Graphics(ok)));
                        } else {
                            // The DA1 behind the answer is this query's: take
                            // it too, a moment at most.
                            *until = Some(now + DETECT_AFTER_DA1);
                        }
                        b = rest;
                        break;
                    }
                    _ => unreachable!(),
                }
            }
            match pattern.partial(&all) {
                Some(p) => {
                    w.carry = all[p..].to_vec();
                    all.truncate(p);
                    b = all;
                }
                None => b = all,
            }
        }
        if !b.is_empty() {
            self.queue.push_back(Input::Data(b));
        }
    }

    /// The next input no call waits for, in the order it arrived.
    pub fn poll_input(&mut self) -> Option<Input> {
        self.queue.pop_front()
    }

    /// Whether inputs are queued.
    pub fn has_input(&self) -> bool {
        !self.queue.is_empty()
    }

    // --- waiting --------------------------------------------------------

    fn wait(&mut self, kind: Kind, what: &'static str, now: u64, timeout: u64) -> Ticket {
        self.next_ticket += 1;
        let ticket = Ticket(self.next_ticket);
        if self.ended || self.closed {
            self.answers.push((
                ticket,
                Err(NoAnswer {
                    what: what.to_string(),
                    replies: Vec::new(),
                }),
            ));
            return ticket;
        }
        self.waits.push(Wait {
            ticket,
            kind,
            what,
            since: now,
            deadline: now + timeout,
            carry: Vec::new(),
        });
        ticket
    }

    fn finish(&mut self, i: usize, result: Result<Answer, NoAnswer>) {
        let mut w = self.waits.remove(i);
        if !w.carry.is_empty() {
            self.queue
                .push_back(Input::Data(std::mem::take(&mut w.carry)));
        }
        self.answers.push((w.ticket, result));
    }

    fn fail_all(&mut self) {
        for i in (0..self.waits.len()).rev() {
            let w = &mut self.waits[i];
            let result = match &mut w.kind {
                Kind::Graphics { answer, .. } => Ok(Answer::Graphics(answer.unwrap_or(false))),
                Kind::Fence(replies) => Err(NoAnswer {
                    what: w.what.to_string(),
                    replies: std::mem::take(replies),
                }),
                _ => Err(NoAnswer {
                    what: w.what.to_string(),
                    replies: Vec::new(),
                }),
            };
            self.finish(i, result);
        }
    }

    /// Takes a waiting call's answer, once it has one: what it waited for,
    /// or [`NoAnswer`] when the time ran out or the input ended.
    pub fn answer(&mut self, ticket: Ticket) -> Option<Result<Answer, NoAnswer>> {
        let i = self.answers.iter().position(|(t, _)| *t == ticket)?;
        Some(self.answers.remove(i).1)
    }

    /// Gives up waiting for a call's answer: one that comes is the input
    /// stream's.
    pub fn forget(&mut self, ticket: Ticket) {
        if let Some(i) = self.waits.iter().position(|w| w.ticket == ticket) {
            let mut w = self.waits.remove(i);
            if !w.carry.is_empty() {
                self.queue
                    .push_back(Input::Data(std::mem::take(&mut w.carry)));
            }
        }
        self.answers.retain(|(t, _)| *t != ticket);
    }

    /// Sends one command numbered with `n` and waits for its reply,
    /// [`REPLY_TIMEOUT`] at most ([`Answer::Reply`]). The number is never
    /// 1, which is the query's. An error reply is a reply, not a
    /// [`NoAnswer`].
    pub fn request(&mut self, command: impl Into<Cmd>, now: u64) -> Ticket {
        self.next_n += 1;
        let n = self.next_n;
        let t = self.wait(Kind::Reply(n), "a request", now, REPLY_TIMEOUT);
        self.send(command.into().n(n));
        t
    }

    /// Waits until the terminal has taken everything written before it (it
    /// sends DA1, which terminals answer in order), [`FENCE_TIMEOUT`] at
    /// most, and answers with the replies nothing read ([`Answer::Fence`]):
    /// the errors of commands sent with `q=1`, mostly. A program calls it
    /// before it exits, so that no reply is left for the shell to read as
    /// typing.
    pub fn fence(&mut self, now: u64) -> Ticket {
        // Replies that came before the fence, and that the input stream
        // has not delivered, are the fence's too.
        let mut replies = Vec::new();
        self.queue.retain(|i| match i {
            Input::Message(m) => match m.reply().filter(|r| r.n.is_none()) {
                Some(r) => {
                    replies.push(r);
                    false
                }
                None => true,
            },
            _ => true,
        });
        let t = self.wait(Kind::Fence(replies), "the fence", now, FENCE_TIMEOUT);
        self.send(DA1);
        t
    }

    /// Writes DA1 and answers with how long the terminal took to answer it
    /// ([`Answer::Da1`]), [`FENCE_TIMEOUT`] at most.
    pub fn ping(&mut self, now: u64) -> Ticket {
        let t = self.await_da1(now, FENCE_TIMEOUT);
        self.send(DA1);
        t
    }

    /// Waits for the next DA1 answer that detection does not take, and
    /// nothing before it takes, `timeout` at most, without writing the
    /// request: a building block for a ping that goes out with a frame.
    pub fn await_da1(&mut self, now: u64, timeout: u64) -> Ticket {
        self.wait(Kind::Da1, "DA1", now, timeout)
    }

    /// Asks where the cursor is (CPR), and answers with its row and column
    /// from 0 ([`Answer::Cursor`]), [`FENCE_TIMEOUT`] at most.
    pub fn cursor_position(&mut self, now: u64) -> Ticket {
        let t = self.wait(Kind::Cursor, "the cursor's position", now, FENCE_TIMEOUT);
        self.send("\x1b[6n");
        t
    }

    /// Asks whether the terminal shows kitty graphics, so that a program on
    /// a terminal that is not a host can suggest a polyfill (SPEC §14):
    /// [`Answer::Graphics`], false for a terminal that answers neither the
    /// query nor DA1 within 1.5 s. A link made with [`Link::known`] is not
    /// asked.
    pub fn kitty_graphics(&mut self, now: u64) -> Ticket {
        if self.known.is_some() {
            self.next_ticket += 1;
            let t = Ticket(self.next_ticket);
            self.answers.push((t, Ok(Answer::Graphics(false))));
            return t;
        }
        let t = self.wait(
            Kind::Graphics {
                answer: None,
                da1: false,
                until: None,
            },
            "the kitty graphics query",
            now,
            DETECT_TIMEOUT,
        );
        self.send(GRAPHICS_QUERY);
        t
    }

    // --- detection ------------------------------------------------------

    /// Asks the terminal whether it is a hotty host that speaks this SDK's
    /// version (SPEC §4), once: later calls do nothing. `late` asks for a
    /// late answer, as [`Link::late`] does. The query goes out with the next
    /// [`Link::take_output`]; [`Link::decided`] and [`Link::detected`] say
    /// how far it got, and [`Link::native`] what it found.
    pub fn detect(&mut self, now: u64, late: bool) {
        if self.started {
            return;
        }
        self.started = true;
        if let Some(k) = &self.known {
            self.native = k.native;
            self.caps.clone_from(&k.caps);
            return;
        }
        let mut d = Detector::new(QUERY_N).late(self.late || late);
        let q = d.start(now);
        self.detector = Some(d);
        self.send(q);
        if self.ended || self.closed {
            if let Some(d) = self.detector.as_mut() {
                d.end(now);
            }
            self.on_detect();
        }
    }

    fn on_detect(&mut self) {
        if let Some(d) = &self.detector
            && d.decided()
        {
            self.native = d.state() == State::Native;
            self.caps = d.caps().cloned();
        }
    }

    /// Whether detection was started ([`Link::detect`]).
    pub fn started(&self) -> bool {
        self.started
    }

    /// Whether detection has decided: the terminal replied to the query, or
    /// it is known not to be a host. A program that must not wait, such as
    /// a framework's adapter, acts on it.
    pub fn decided(&self) -> bool {
        self.started && self.detector.as_ref().is_none_or(Detector::decided)
    }

    /// Whether detection is done: nothing more of it will arrive, so a
    /// program that hands the terminal on may.
    pub fn detected(&self) -> bool {
        self.started && self.detector.as_ref().is_none_or(Detector::done)
    }

    /// What detection found: true for a hotty host. False until it is
    /// decided; true from a late answer on.
    pub fn native(&self) -> bool {
        self.native
    }

    /// The host's capabilities, when native, kept current by the `caps`
    /// events it has sent since ([`Caps::merge`]).
    pub fn caps(&self) -> Option<&Caps> {
        self.caps.as_ref()
    }

    /// Withdraws the query, when it asked for a late answer and none has
    /// come (SPEC §4): returns the command to write, or `""` when there is
    /// nothing to withdraw. [`Link::close`] queues it itself; an adapter
    /// that writes with its frames, as a Session does, writes what this
    /// returns.
    pub fn withdraw_late(&mut self) -> String {
        if !self.late_pending() {
            return String::new();
        }
        self.withdrawn = true;
        withdraw_late()
    }

    // --- names ----------------------------------------------------------

    /// A surface name of this program's own: the link's name, a dash, and
    /// `name`, made a valid surface name, so that two runs never share a
    /// surface. Every name it returns is remembered ([`Link::surfaces`]).
    pub fn surface(&mut self, name: &str) -> String {
        let s = surface_name(&format!("{}-{name}", self.name));
        if !self.names.contains(&s) {
            self.names.push(s.clone());
        }
        s
    }

    /// The names [`Link::surface`] has returned: what this program may have
    /// left on the terminal.
    pub fn surfaces(&self) -> &[String] {
        &self.names
    }

    /// Detaches every surface [`Link::surface`] has named (SPEC §5.5), so
    /// that none reports to whatever reads the terminal after this program.
    pub fn detach_all(&mut self) {
        let cmds: String = self.names.iter().map(|s| detach(s).to_string()).collect();
        self.send(cmds);
    }

    // --- closing --------------------------------------------------------

    /// Whether the link is closed.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Closes the link: calls that wait give up, and the input stream ends.
    /// It queues nothing but the withdrawal of a late query no answer came
    /// to: what is left on the screen, and the replies not read, are the
    /// program's to settle first (SDK.md §2.6).
    pub fn close(&mut self, now: u64) {
        if self.closed {
            return;
        }
        let w = self.withdraw_late();
        self.send(w);
        self.closed = true;
        self.esc_at = None;
        if let Some(d) = self.detector.as_mut().filter(|d| !d.done()) {
            d.end(now);
            self.on_detect();
        }
        self.fail_all();
    }
}

#[derive(Clone, Copy)]
enum Pattern {
    Cursor,
    Graphics,
}

enum Found {
    Cursor(u32, u32),
    Graphics(bool),
}

impl Pattern {
    // A complete answer in b: its start, its end and its value.
    fn find(self, b: &[u8]) -> Option<(usize, usize, Found)> {
        let mut i = 0;
        while let Some(p) = b[i..].iter().position(|&c| c == 0x1b) {
            let s = i + p;
            if let Some((end, v)) = self.at(&b[s..]) {
                return Some((s, s + end, v));
            }
            i = s + 1;
        }
        None
    }

    // An answer at the start of b: its length and value.
    fn at(self, b: &[u8]) -> Option<(usize, Found)> {
        match self {
            Pattern::Cursor => {
                let rest = b.strip_prefix(b"\x1b[")?;
                let (row, rest) = digits(rest)?;
                let rest = rest.strip_prefix(b";")?;
                let (col, rest) = digits(rest)?;
                rest.strip_prefix(b"R")?;
                let len = b.len() - rest.len() + 1;
                Some((
                    len,
                    Found::Cursor(row.saturating_sub(1), col.saturating_sub(1)),
                ))
            }
            Pattern::Graphics => {
                let rest = b.strip_prefix(b"\x1b_Gi=31;")?;
                let end = rest.iter().position(|&c| c == 0x1b)?;
                if rest.get(end + 1) != Some(&b'\\') {
                    return None;
                }
                let ok = &rest[..end] == b"OK";
                Some((b.len() - rest.len() + end + 2, Found::Graphics(ok)))
            }
        }
    }

    // Where b ends with what may still become an answer, to hold.
    fn partial(self, b: &[u8]) -> Option<usize> {
        let s = b.iter().rposition(|&c| c == 0x1b)?;
        let tail = &b[s..];
        let ok = match self {
            Pattern::Cursor => {
                let t = &tail[1..];
                t.is_empty()
                    || (t[0] == b'['
                        && t[1..].iter().all(|c| c.is_ascii_digit() || *c == b';')
                        && t[1..].iter().filter(|c| **c == b';').count() <= 1)
            }
            Pattern::Graphics => {
                let prefix = b"\x1b_Gi=31;";
                let n = tail.len().min(prefix.len());
                tail[..n] == prefix[..n]
            }
        };
        // An ESC inside a graphics answer is its ST's: look one further back.
        if !ok
            && matches!(self, Pattern::Graphics)
            && tail.len() == 1
            && let Some(s2) = b[..s].iter().rposition(|&c| c == 0x1b)
            && b[s2..].starts_with(b"\x1b_Gi=31;")
        {
            return Some(s2);
        }
        ok.then_some(s)
    }
}

fn digits(b: &[u8]) -> Option<(u32, &[u8])> {
    let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 || n > 9 {
        return None;
    }
    let v = std::str::from_utf8(&b[..n]).ok()?.parse().ok()?;
    Some((v, &b[n..]))
}
