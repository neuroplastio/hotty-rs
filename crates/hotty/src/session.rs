//! A full-screen program's side of hotty, for a UI framework's adapter
//! (SDK.md §4.3). It finds out whether the terminal is a host, keeps the
//! program's surfaces on screen as its frame changes, and turns what the
//! host sends into messages. It assumes nothing of the framework, and does
//! no I/O: it stands on a [`Link`] the adapter feeds, and hands back the
//! bytes to write.
//!
//! - **Input.** [`Session::poll`] reads what the link was handed and returns
//!   [`SessionMessage`]s: ready (the mode is known), an event, an error (a
//!   command refused), an ack (the ok of a numbered command), relayout,
//!   pong, and input that is not hotty's, as it was.
//! - **Frames.** With each frame, the program says which surfaces it wants
//!   where ([`Session::layout`]); [`Session::flush`] returns the commands
//!   that changed, for the adapter to write with that frame, in the same
//!   stream, so that a placement lands with the cells around it.
//! - **Relayout.** A framework's renderer erases, scrolls and switches
//!   screens on its own, and a host drops placements with them (SPEC §5.4).
//!   The adapter hands the Session what the framework writes
//!   ([`Session::watch`]), or says when it redrew ([`Session::relayout`]),
//!   and the Session asks for a new layout.
//!
//! In a terminal that is not a host the mode is [`Mode::Text`], and the
//! Session sends nothing: the program draws everything in cells.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;

use crate::code;
use crate::commands::{Placement, Window, del, detach, doc, hide, place_at};
use crate::link::{Answer, Input, Link, Ticket};
use crate::messages::{Caps, Event, Reply};

/// How many surfaces a Session keeps at most, shown and hidden, unless its
/// limit or the host says fewer: a hidden surface keeps its memory in the
/// terminal.
pub const DEFAULT_LIMIT: usize = 48;

/// How long a ping waits for a frame to go with before it goes on its own,
/// in milliseconds.
pub const PING_WAIT: u64 = 100;

/// What the terminal turned out to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Not known yet: layout and send do nothing.
    #[default]
    Detecting,
    /// A hotty host: surfaces.
    Native,
    /// Not a host: everything is drawn in cells.
    Text,
}

/// A rectangle of cells, from the screen's top-left corner (0, 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// The first column; negative where it reaches past the screen's left.
    pub x: i32,
    /// The first row; negative where it reaches past the screen's top.
    pub y: i32,
    /// The columns.
    pub w: i32,
    /// The rows.
    pub h: i32,
}

impl Rect {
    /// A rectangle.
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// The part of `self` inside `o`; `w` or `h` is 0 or less if none is.
    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (
            (self.x + self.w).min(o.x + o.w),
            (self.y + self.h).min(o.y + o.h),
        );
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Whether it has no cells.
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
}

/// A surface the program wants on screen, in one frame.
pub struct Want<'a> {
    /// The surface's name: unique, and valid ([`valid_name`](crate::valid_name)).
    pub name: String,
    /// The whole surface, in screen cells. With `clip` it may reach past
    /// the screen's edges.
    pub rect: Rect,
    /// The part of the screen the surface shows in, such as a scrolling
    /// region: the window of it inside is placed (SPEC §5.2). A surface with
    /// nothing inside is as good as not wanted.
    pub clip: Option<Rect>,
    /// When the program stops wanting it, hide it instead of deleting it,
    /// so it shows again without its document being sent (SPEC §5.4).
    pub keep: bool,
    /// Above (greater) or below the surfaces it overlaps (SPEC §5.2).
    pub z: i32,
    /// A `press` event wherever the user presses in it (SPEC §9). Press,
    /// fit and hover belong to the surface on the host (SPEC §5.2), so the
    /// Session sends each only when it differs from what the surface last
    /// had.
    pub press: bool,
    /// A `fit` event whenever the rows its document needs change (SPEC §5.2).
    pub fit: bool,
    /// A `hover` event each time the element under the pointer changes
    /// (SPEC §9.4).
    pub hover: bool,
    /// The axes the host scrolls the document along itself (SPEC §5.3): a
    /// bitmask of [`SCROLL_VERTICAL`](crate::SCROLL_VERTICAL) and
    /// [`SCROLL_HORIZONTAL`](crate::SCROLL_HORIZONTAL). It goes out with the
    /// document, so a change sends the document again.
    pub scroll: u32,
    /// Returns the document. Called only when the document must be sent:
    /// the first time, again after the surface was deleted or the host lost
    /// it, and when its `scroll` changed.
    pub doc: Box<dyn FnOnce() -> String + 'a>,
}

impl<'a> Want<'a> {
    /// A surface named `name` on `rect`, its document from `doc`, with
    /// nothing else asked of it.
    pub fn new(name: impl Into<String>, rect: Rect, doc: impl FnOnce() -> String + 'a) -> Want<'a> {
        Want {
            name: name.into(),
            rect,
            clip: None,
            keep: false,
            z: 0,
            press: false,
            fit: false,
            hover: false,
            scroll: 0,
            doc: Box::new(doc),
        }
    }
}

impl fmt::Debug for Want<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Want")
            .field("name", &self.name)
            .field("rect", &self.rect)
            .field("clip", &self.clip)
            .field("keep", &self.keep)
            .field("z", &self.z)
            .field("press", &self.press)
            .field("fit", &self.fit)
            .field("hover", &self.hover)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

/// What the Session tells the program.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // passed now and then, not stored in bulk
pub enum SessionMessage {
    /// Detection decided, once; again, native, when a late answer comes.
    Ready {
        /// What the terminal is.
        mode: Mode,
        /// The host's capabilities, when native.
        caps: Option<Caps>,
    },
    /// Something the user did in a surface (SPEC §9).
    Event(Event),
    /// A command the host refused that the Session does not handle itself.
    Error(Reply),
    /// The host's ok for a command the program numbered: it has carried it
    /// out. A reply reaches the program the way the user's keys do, in
    /// order (SPEC §3.6), so every key typed before the host carried it out
    /// came before its ack.
    Ack(Reply),
    /// Lay the surfaces out again: the screen was erased or scrolled under
    /// them, or a document went missing.
    Relayout,
    /// A ping's answer: the terminal took what was written before it,
    /// `took` milliseconds after.
    Pong {
        /// Milliseconds from the ping's write to its answer.
        took: u64,
    },
    /// Bytes that are not hotty's: keys, the mouse, other answers, as they
    /// were.
    Input(Vec<u8>),
}

/// What [`Session::layout`] has sent so far, and the relayouts after the
/// screen was erased or scrolled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Documents sent.
    pub docs: usize,
    /// Placements sent.
    pub places: usize,
    /// Hides sent.
    pub hides: usize,
    /// Deletes sent.
    pub deletes: usize,
    /// Relayouts asked for.
    pub relayouts: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Flags {
    press: bool,
    fit: bool,
    hover: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    at: Rect,
    cols: i32,
    rows: i32,
    win: Rect,
    z: i32,
    flags: Flags,
}

/// A full-screen program's side of hotty (SDK.md §4.3), sans I/O.
#[derive(Debug, Default)]
pub struct Session {
    mode: Mode,
    caps: Option<Caps>,
    limit: usize,
    late: bool,
    sent: usize,
    counts: Counts,
    placed: HashMap<String, Placed>,
    flags: HashMap<String, Flags>,
    has_doc: BTreeSet<String>,
    scroll: HashMap<String, u32>,
    keep: HashMap<String, bool>,
    seen: HashMap<String, u64>,
    refused: BTreeSet<String>,
    layouts: u64,
    learned: usize,
    closed: bool,
    out: String,
    messages: VecDeque<SessionMessage>,
    relayout_due: bool,
    ping_at: Option<u64>,
    pings: Vec<Ticket>,
    watcher: Watcher,
}

fn changed(was: bool, now: bool) -> Option<bool> {
    (was != now).then_some(now)
}

impl Session {
    /// A Session with the default limit.
    pub fn new() -> Session {
        Session::default()
    }

    /// The most surfaces to keep, shown and hidden; [`DEFAULT_LIMIT`] when
    /// 0. The host's `limits.surfaces`, and one an `EQUOTA` taught, lower
    /// it further, and those are hard: a surface beyond them waits for room.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Session {
        self.limit = limit;
        self
    }

    /// Ask for a late answer (SPEC §4): if one comes while the mode is
    /// text, the mode becomes native, ready comes again, and then relayout.
    /// A program that asks for it draws either rendition whenever the mode
    /// says. Until it comes, [`Session::close`] withdraws the query.
    #[must_use]
    pub fn late(mut self, late: bool) -> Session {
        self.late = late;
        self
    }

    /// What detection found.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The host's capabilities, when native, kept current by the `caps`
    /// events it has sent since ([`Caps::merge`]).
    pub fn caps(&self) -> Option<&Caps> {
        self.caps.as_ref()
    }

    /// What layout has sent.
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The bytes of hotty commands [`Session::flush`] has returned.
    pub fn sent(&self) -> usize {
        self.sent
    }

    /// Asks the terminal whether it is a host (SPEC §4), through the link:
    /// call it once the program starts. [`SessionMessage::Ready`] comes once
    /// it is decided. The query goes out with the link's output.
    pub fn detect(&mut self, link: &mut Link, now: u64) {
        link.detect(now, self.late);
    }

    /// The next message for the program, from what the link was handed, or
    /// `None` when there is none yet. Call it until it returns `None` after
    /// handing the link what the terminal sent, and when the time passes
    /// [`Session::deadline`].
    pub fn poll(&mut self, link: &mut Link, now: u64) -> Option<SessionMessage> {
        if self.ping_at.is_some_and(|at| now >= at) {
            let da1 = self.ping_now(link, now);
            link.send(da1);
        }
        let mut i = 0;
        while i < self.pings.len() {
            match link.answer(self.pings[i]) {
                Some(a) => {
                    self.pings.remove(i);
                    if let Ok(Answer::Da1 { took }) = a {
                        self.messages.push_back(SessionMessage::Pong { took });
                    }
                }
                None => i += 1,
            }
        }
        if self.mode == Mode::Detecting && link.decided() && !self.closed {
            let native = link.native();
            self.mode = if native { Mode::Native } else { Mode::Text };
            self.caps = if native { link.caps().cloned() } else { None };
            self.messages.push_back(SessionMessage::Ready {
                mode: self.mode,
                caps: self.caps.clone(),
            });
        }
        loop {
            if let Some(m) = self.messages.pop_front() {
                if m == SessionMessage::Relayout {
                    if !self.relayout_due {
                        continue;
                    }
                    self.relayout_due = false;
                }
                return Some(m);
            }
            let input = link.poll_input()?;
            match input {
                Input::Data(b) => return Some(SessionMessage::Input(b)),
                Input::Host(caps) => self.late_answer(caps),
                Input::Message(m) => {
                    if let Some(r) = m.reply() {
                        self.reply(r);
                    } else if let Some(ev) = m.event() {
                        if ev.kind == crate::kind::CAPS
                            && self.mode == Mode::Native
                            && let (Some(ch), Some(caps)) = (ev.changed(), self.caps.as_ref())
                        {
                            self.caps = Some(caps.merge(ch));
                        }
                        return Some(SessionMessage::Event(ev));
                    }
                }
            }
        }
    }

    /// When to call [`Session::poll`] again with no input: a ping that waits
    /// for a frame going on its own. The link's own deadline
    /// ([`Link::deadline`]) is the link's.
    pub fn deadline(&self) -> Option<u64> {
        self.ping_at
    }

    // A late answer (SPEC §4): a host attached after the Session fell back
    // to text. Ready again, and a layout.
    fn late_answer(&mut self, caps: Option<Caps>) {
        if self.mode == Mode::Native || self.closed {
            return;
        }
        self.mode = Mode::Native;
        self.caps = caps;
        self.messages.push_back(SessionMessage::Ready {
            mode: Mode::Native,
            caps: self.caps.clone(),
        });
        self.relayout_due = true;
        self.messages.push_back(SessionMessage::Relayout);
    }

    fn forget(&mut self, s: &str) {
        self.has_doc.remove(s);
        self.placed.remove(s);
        self.flags.remove(s);
    }

    fn reply(&mut self, r: Reply) {
        if r.ok {
            // A command the program numbered; queries are the link's.
            if r.n.is_some() && r.re != "q" {
                self.messages.push_back(SessionMessage::Ack(r));
            }
            return;
        }
        let s = r.surface.clone().unwrap_or_default();
        let code = r.code.as_deref().unwrap_or("");
        if r.re == "doc" && code == code::EQUOTA && self.has_doc.contains(&s) {
            // Over the host's limit: it holds fewer surfaces than the
            // Session has sent. Keep below that from now on, and lay out
            // again. The placement sent with the document fails too.
            let n = self.has_doc.len().saturating_sub(1).max(1);
            if self.learned == 0 || n < self.learned {
                self.learned = n;
            }
            self.forget(&s);
            self.refused.insert(s);
            self.relayout_due = true;
            self.messages.push_back(SessionMessage::Relayout);
            return;
        }
        if r.re == "place" && code == code::ENOENT && self.refused.remove(&s) {
            return;
        }
        if r.re == "place" && code == code::ENOENT && self.has_doc.contains(&s) {
            // The host lost the document (a reset, the alternate screen):
            // send both again with the next layout.
            self.forget(&s);
            self.relayout_due = true;
            self.messages.push_back(SessionMessage::Relayout);
            return;
        }
        self.messages.push_back(SessionMessage::Error(r));
    }

    /// Makes the surfaces on screen match `wants`: a new surface gets its
    /// document (attached, `q=1`, with its scroll) and a placement; one
    /// whose scroll changed gets its document again, and keeps its
    /// placement; a moved or clipped one is placed again (only the window
    /// inside its clip); one no longer wanted is hidden if it is kept, or
    /// else deleted. What it sends leaves with the next flush. It does
    /// nothing until the mode is native, and after close.
    pub fn layout<'a>(&mut self, wants: impl IntoIterator<Item = Want<'a>>) {
        if self.mode != Mode::Native || self.closed {
            return;
        }
        self.layouts += 1;
        self.relayout_due = false;
        let mut on = Vec::new();
        let mut wanted = BTreeSet::new();
        for w in wants {
            let at = w.clip.map_or(w.rect, |c| w.rect.intersect(&c));
            if at.is_empty() || at.x < 0 || at.y < 0 {
                continue;
            }
            wanted.insert(w.name.clone());
            self.keep.insert(w.name.clone(), w.keep);
            self.seen.insert(w.name.clone(), self.layouts);
            on.push((w, at));
        }
        for (w, at) in on {
            let has = self.has_doc.contains(&w.name);
            if !has || self.scroll.get(&w.name) != Some(&w.scroll) {
                if !has && !self.make_room(&wanted) {
                    continue; // the host holds no more: not until one goes
                }
                let html = (w.doc)();
                self.queue(doc(&w.name, &html).scroll(w.scroll));
                self.counts.docs += 1;
                self.has_doc.insert(w.name.clone());
                self.scroll.insert(w.name.clone(), w.scroll);
                self.refused.remove(&w.name);
                // A replaced document keeps its placement (SPEC §5.1).
                if !has {
                    self.placed.remove(&w.name);
                    self.flags.remove(&w.name); // a new surface starts from none
                }
            }
            let p = Placed {
                at,
                cols: w.rect.w,
                rows: w.rect.h,
                win: Rect::new(at.x - w.rect.x, at.y - w.rect.y, at.w, at.h),
                z: w.z,
                flags: Flags {
                    press: w.press,
                    fit: w.fit,
                    hover: w.hover,
                },
            };
            if self.placed.get(&w.name) != Some(&p) {
                let have = self.flags.get(&w.name).copied().unwrap_or_default();
                let placement = Placement {
                    cols: p.cols as u32,
                    rows: p.rows as u32,
                    window: Some(Window {
                        x: p.win.x as u32,
                        y: p.win.y as u32,
                        w: p.win.w as u32,
                        h: p.win.h as u32,
                    }),
                    z: p.z,
                    press: changed(have.press, p.flags.press),
                    fit: changed(have.fit, p.flags.fit),
                    hover: changed(have.hover, p.flags.hover),
                    keep_cursor: true,
                };
                self.queue(place_at(&w.name, at.x as u32, at.y as u32, &placement));
                self.counts.places += 1;
                self.placed.insert(w.name.clone(), p);
                self.flags.insert(w.name.clone(), p.flags);
            }
        }
        let names: Vec<String> = self.has_doc.iter().cloned().collect();
        for name in names {
            if wanted.contains(&name) {
                continue;
            }
            if self.keep.get(&name).copied().unwrap_or(false) {
                if self.placed.remove(&name).is_some() {
                    self.queue(hide(&name));
                    self.counts.hides += 1;
                }
            } else {
                self.delete(&name);
            }
        }
    }

    // The most surfaces to have at once, and whether it is the host's.
    fn limit_now(&self) -> (usize, bool) {
        let mut n = if self.limit > 0 {
            self.limit
        } else {
            DEFAULT_LIMIT
        };
        let mut hard = false;
        let l = self
            .caps
            .as_ref()
            .and_then(|c| c.limit("surfaces"))
            .unwrap_or(0);
        if l > 0 && (l as usize) <= n {
            (n, hard) = (l as usize, true);
        }
        if self.learned > 0 && self.learned <= n {
            (n, hard) = (self.learned, true);
        }
        (n, hard)
    }

    // Deletes surfaces the program no longer wants, the ones seen longest
    // ago first, until one more fits the limit. A surface wanted now is
    // never taken. Returns whether one more may be sent: only the host's
    // own limit refuses one.
    fn make_room(&mut self, wanted: &BTreeSet<String>) -> bool {
        let (n, hard) = self.limit_now();
        while self.has_doc.len() >= n {
            let oldest = self
                .has_doc
                .iter()
                .filter(|s| !wanted.contains(*s))
                .min_by_key(|s| self.seen.get(*s).copied().unwrap_or(0))
                .cloned();
            match oldest {
                Some(s) => self.delete(&s),
                None => return !hard,
            }
        }
        true
    }

    /// Deletes a surface, kept or not: one that will not come back. It
    /// leaves with the next flush.
    pub fn delete(&mut self, name: &str) {
        if self.has_doc.contains(name) {
            self.queue(del(name));
            self.counts.deletes += 1;
        }
        self.forget(name);
        self.scroll.remove(name);
        self.keep.remove(name);
        self.seen.remove(name);
    }

    /// Whether the host has a surface's document, on screen or hidden, so
    /// that deltas to it make sense.
    pub fn has(&self, name: &str) -> bool {
        self.has_doc.contains(name)
    }

    /// Whether a surface is on screen: placed by the last layout, and not
    /// hidden since.
    pub fn placed(&self, name: &str) -> bool {
        self.placed.contains_key(name)
    }

    fn queue(&mut self, c: impl fmt::Display) {
        use fmt::Write as _;
        let _ = write!(self.out, "{c}");
    }

    /// Queues commands (deltas, focus) for the next flush. Nothing when the
    /// terminal is not a host, or after close.
    pub fn send(&mut self, commands: impl fmt::Display) {
        if self.mode == Mode::Native && !self.closed {
            self.queue(commands);
        }
    }

    /// What is queued, to write with the frame: empty when nothing is. A
    /// ping waiting goes at its end, so that its pong says the terminal took
    /// the frame written before it.
    pub fn flush(&mut self, link: &mut Link, now: u64) -> String {
        let mut s = std::mem::take(&mut self.out);
        self.sent += s.len();
        if self.ping_at.is_some() {
            s.push_str(&self.ping_now(link, now));
        }
        s
    }

    /// Asks the terminal to answer (DA1) with the next flush: its pong says
    /// it has taken what was written before it, so a program can measure
    /// what a frame costs. One at a time. Without a flush within
    /// [`PING_WAIT`], the ping goes on its own, at the next poll.
    pub fn ping(&mut self, now: u64) {
        if self.ping_at.is_none() && !self.closed {
            self.ping_at = Some(now + PING_WAIT);
        }
    }

    fn ping_now(&mut self, link: &mut Link, now: u64) -> String {
        self.ping_at = None;
        self.pings
            .push(link.await_da1(now, crate::link::FENCE_TIMEOUT));
        "\x1b[c".to_string()
    }

    /// Reads what the framework writes on its way out, a piece at a time,
    /// as it goes, and asks for a new layout ([`SessionMessage::Relayout`])
    /// after erase in display (2 or 3), scroll up or down, insert or delete
    /// lines, a new scrolling region, reverse index, leaving the alternate
    /// screen, or a full reset. A full reset deletes every surface, so after
    /// one the next layout sends the documents again. The surfaces count as
    /// placed nowhere from that point in the stream, so a layout that
    /// follows places them again.
    pub fn watch(&mut self, output: &[u8]) {
        match self.watcher.scan(output) {
            Seen::Nothing => {}
            Seen::Relayout => self.dropped(false),
            Seen::Reset => self.dropped(true),
        }
    }

    /// The bytes [`Session::watch`] has read.
    pub fn watched(&self) -> usize {
        self.watcher.written
    }

    /// The environment redrew, so the host may have dropped placements: an
    /// environment that knows its own redraws (an editor) says so here
    /// instead of handing over its output.
    pub fn relayout(&mut self) {
        self.dropped(false);
    }

    fn dropped(&mut self, reset: bool) {
        if self.mode != Mode::Native || self.closed {
            return;
        }
        if reset {
            // A full reset deletes every surface (SPEC §5.4), hidden ones
            // too: the next layout sends their documents again.
            self.has_doc.clear();
            self.flags.clear();
            self.refused.clear();
        }
        if self.placed.is_empty() {
            return;
        }
        self.placed.clear();
        self.counts.relayouts += 1;
        if !self.relayout_due {
            self.relayout_due = true;
            self.messages.push_back(SessionMessage::Relayout);
        }
    }

    /// Detaches every surface the Session has sent (SPEC §5.5), for a
    /// program that leaves them on the screen when it quits, rather than
    /// close. They stop reporting to whatever reads the terminal after the
    /// program. It leaves with the next flush.
    pub fn detach_all(&mut self) {
        let names: Vec<String> = self.has_doc.iter().cloned().collect();
        for n in names {
            self.send(detach(&n));
        }
    }

    /// Ends the Session on the program's way out: it deletes every surface
    /// it sent, and none of anyone else's, and returns what writes the
    /// deletes, and the withdrawal of a late query no answer came to (SPEC
    /// §4). After it, layout and send do nothing, so that a frame drawn
    /// before the program quits does not send a document again.
    pub fn close(&mut self, link: &mut Link, now: u64) -> String {
        let names: Vec<String> = self.has_doc.iter().cloned().collect();
        for n in names {
            self.delete(&n);
        }
        self.ping_at = None;
        let mut out = self.flush(link, now);
        out.push_str(&link.withdraw_late());
        self.closed = true;
        out
    }

    /// The surfaces sent, shown or hidden, by name.
    pub fn surfaces(&self) -> impl Iterator<Item = &str> {
        self.has_doc.iter().map(String::as_str)
    }

    /// The rectangles of the surfaces on screen, by name: where the last
    /// layout placed their windows.
    pub fn placements(&self) -> BTreeMap<&str, Rect> {
        self.placed
            .iter()
            .map(|(k, p)| (k.as_str(), p.at))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Nothing,
    Relayout,
    Reset,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Ground,
    Esc,
    Csi,
    Str,
}

// Scans output for what can take placements with it, keeping its parser
// state across writes, since a sequence can be split between two.
#[derive(Clone, Debug, Default)]
struct Watcher {
    written: usize,
    state: Scan,
    private: u8,
    param: Vec<u8>,
    str_esc: bool,
}

impl Watcher {
    fn scan(&mut self, p: &[u8]) -> Seen {
        let (mut hit, mut reset) = (false, false);
        self.written += p.len();
        for &c in p {
            match self.state {
                Scan::Ground => {
                    if c == 0x1b {
                        self.state = Scan::Esc;
                    }
                }
                Scan::Esc => match c {
                    b'[' => {
                        self.state = Scan::Csi;
                        self.private = 0;
                        self.param.clear();
                    }
                    b']' | b'P' | b'_' | b'^' | b'X' => {
                        self.state = Scan::Str;
                        self.str_esc = false;
                    }
                    _ => {
                        // RI scrolls at the top margin; RIS resets everything.
                        match c {
                            b'M' => hit = true,
                            b'c' => reset = true,
                            _ => {}
                        }
                        self.state = Scan::Ground;
                    }
                },
                Scan::Csi => {
                    if (0x40..=0x7e).contains(&c) {
                        let p = self.param.as_slice();
                        if self.private == 0 {
                            match c {
                                b'J' => hit |= p == b"2" || p == b"3",
                                b'S' | b'T' | b'L' | b'M' | b'r' => hit = true,
                                _ => {}
                            }
                        } else if self.private == b'?' && c == b'l' {
                            // Leaving the alternate screen.
                            hit |= p
                                .split(|&b| b == b';')
                                .any(|m| m == b"1049" || m == b"1047" || m == b"47");
                        }
                        self.state = Scan::Ground;
                    } else if matches!(c, b'?' | b'>' | b'=' | b'<') {
                        self.private = c;
                    } else if self.param.len() < 32 {
                        self.param.push(c);
                    }
                }
                Scan::Str => {
                    if c == 0x07 || (self.str_esc && c == b'\\') {
                        self.state = Scan::Ground;
                    } else {
                        self.str_esc = c == 0x1b;
                    }
                }
            }
        }
        if reset {
            Seen::Reset
        } else if hit {
            Seen::Relayout
        } else {
            Seen::Nothing
        }
    }
}
