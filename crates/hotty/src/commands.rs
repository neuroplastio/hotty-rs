//! The builders: one for each action (SDK.md §3.4).
//!
//! Each returns a command that formats as its escape sequences
//! ([`fmt::Display`]), ready to write like any other output. The reply
//! options are builder methods: [`Cmd::n`] numbers a command and asks for
//! its reply, [`Cmd::q`] sets its quiet level, and a `q` given wins over the
//! level `n` implies, in whatever order they are given (SDK.md §3.4.2).
//!
//! ```
//! use hotty::{Placement, doc, place, set_text};
//!
//! let out = format!(
//!     "{}{}",
//!     doc("clock", "<p id=t>12:00</p>"),
//!     place("clock", &Placement::new(20, 1)),
//! );
//! let tick = set_text("clock", "t", "12:01").to_string();
//! # assert!(out.starts_with("\x1b]7279;a=doc:s=clock;"));
//! # assert!(tick.starts_with("\x1b]7279;a=delta:s=clock:q=2;"));
//! ```

use std::fmt;

use crate::VERSION;
use crate::control::Control;
use crate::encode::encode;
use crate::msgpack::Writer;

/// How much of a reply a command asks for (SPEC §3.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quiet {
    /// Reply whatever the outcome: `q=0`.
    ReplyAlways = 0,
    /// Reply only on error: `q=1`.
    ReplyOnError = 1,
    /// Never reply: `q=2`.
    NoReply = 2,
}

/// One command, as a builder makes it: formatting it ([`fmt::Display`])
/// writes its sequences. [`Cmd::n`] and [`Cmd::q`] are its reply options
/// (SDK.md §3.4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "a command does nothing until it is written"]
pub struct Cmd {
    control: Control,
    payload: Vec<u8>,
    default_q: Quiet,
    n: Option<u64>,
    q: Option<Quiet>,
    detached: bool,
    scroll: u32,
    at: Option<(u32, u32)>,
}

impl Cmd {
    fn new(control: Control, payload: impl Into<Vec<u8>>, default_q: Quiet) -> Cmd {
        Cmd {
            control,
            payload: payload.into(),
            default_q,
            n: None,
            q: None,
            detached: false,
            scroll: 0,
            at: None,
        }
    }

    /// Numbers the command and asks for its reply (`n=<n>`, and quiet
    /// level 0 unless [`Cmd::q`] says otherwise): the reply echoes `n`, so
    /// the program can tell which command it answers.
    pub fn n(mut self, n: u64) -> Cmd {
        self.n = Some(n);
        self
    }

    /// Sets the quiet level. It wins over the level [`Cmd::n`] implies.
    pub fn q(mut self, q: Quiet) -> Cmd {
        self.q = Some(q);
        self
    }

    /// The command's control as it goes out: its keys, then `d`, `scroll`,
    /// `n` and `q` where they apply. `q` goes out only where the wire does
    /// not imply it: 0 with `n`, 1 without (SPEC §3.5).
    pub fn control(&self) -> Control {
        let mut c = self.control.clone();
        if self.detached {
            c.set("d", "1");
        }
        if self.scroll != 0 {
            c.set("scroll", self.scroll.to_string());
        }
        let implied = if self.n.is_some() {
            Quiet::ReplyAlways
        } else {
            Quiet::ReplyOnError
        };
        let q = self.q.unwrap_or(if self.n.is_some() {
            Quiet::ReplyAlways
        } else {
            self.default_q
        });
        if let Some(n) = self.n {
            c.set("n", n.to_string());
        }
        if q != implied {
            c.set("q", (q as u8).to_string());
        }
        c
    }

    /// The command's payload: a document, a delta's ops as msgpack, ….
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((x, y)) = self.at {
            write!(f, "\x1b7\x1b[{};{}H", u64::from(y) + 1, u64::from(x) + 1)?;
        }
        f.write_str(&encode(&self.control(), &self.payload))?;
        if self.at.is_some() {
            f.write_str("\x1b8")?;
        }
        Ok(())
    }
}

impl From<Cmd> for String {
    fn from(c: Cmd) -> String {
        c.to_string()
    }
}

/// [`doc`]'s command: a [`Cmd`] with the document's own options,
/// [`Doc::detached`] and [`Doc::scroll`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "a command does nothing until it is written"]
pub struct Doc(Cmd);

impl Doc {
    /// Numbers the command and asks for its reply, as [`Cmd::n`].
    pub fn n(self, n: u64) -> Doc {
        Doc(self.0.n(n))
    }

    /// Sets the quiet level, as [`Cmd::q`].
    pub fn q(self, q: Quiet) -> Doc {
        Doc(self.0.q(q))
    }

    /// Sends a document the program only shows (`d=1`, SPEC §5.5): the
    /// surface is created detached, or its document replaced and the surface
    /// detached, in the one command. It reports nothing, never takes the
    /// keyboard, and its controls act disabled: what a command prints among
    /// its output, which outlives it.
    pub fn detached(mut self) -> Doc {
        self.0.detached = true;
        self
    }

    /// The axes along which the document scrolls, as a page does (`scroll`,
    /// SPEC §5.1, §5.3): a bitmask of [`SCROLL_VERTICAL`](crate::SCROLL_VERTICAL)
    /// and [`SCROLL_HORIZONTAL`](crate::SCROLL_HORIZONTAL). 0, the default,
    /// sends no key, and the document does not scroll; any other value goes
    /// out as given, for the host to judge.
    pub fn scroll(mut self, axes: u32) -> Doc {
        self.0.scroll = axes;
        self
    }
}

impl fmt::Display for Doc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Doc> for Cmd {
    fn from(d: Doc) -> Cmd {
        d.0
    }
}

impl From<Doc> for String {
    fn from(d: Doc) -> String {
        d.to_string()
    }
}

/// [`query`]'s command: the query and the DA1 that fences it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "a command does nothing until it is written"]
pub struct Query {
    n: u64,
    late: bool,
}

impl Query {
    /// Asks for a late answer (`late=1`, SPEC §4): what passes hotty through
    /// and has no host yet, such as a multiplexer's pane with no terminal
    /// attached, may answer once there is one. [`withdraw_late`] takes it
    /// back.
    pub fn late(mut self) -> Query {
        self.late = true;
        self
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut c = Control::from([("a", "q")]);
        c.push("n", self.n.to_string());
        c.push("v", VERSION);
        if self.late {
            c.push("late", "1");
        }
        f.write_str(&encode(&c, b""))?;
        f.write_str("\x1b[c")
    }
}

impl From<Query> for String {
    fn from(q: Query) -> String {
        q.to_string()
    }
}

/// Asks whether the terminal is a hotty host that speaks
/// [`VERSION`](crate::VERSION), fenced by Primary Device Attributes (SPEC §4):
/// `a=q:n=<n>:v=0.5`, then `ESC [ c`. A host replies to the query, numbered
/// `n`, before the DA1 answer every terminal sends: its capabilities, or
/// `EVERSION` when it speaks no version the query lists. A
/// [`Detector`](crate::Detector) sends it and reads the answers.
pub fn query(n: u64) -> Query {
    Query { n, late: false }
}

/// Withdraws a query that asked for a late answer (SPEC §4): `a=q:q=2`, a
/// query that wants no answer and takes the held one's place, so that no
/// answer reaches whatever reads the terminal next. It has no fence.
pub fn withdraw_late() -> String {
    encode(&Control::from([("a", "q"), ("q", "2")]), b"")
}

/// Creates a surface, or replaces its document (SPEC §5.1). The surface is
/// the program's, even one it had detached, unless the document is sent
/// [detached](Doc::detached). Answered on error (`q=1`): `EQUOTA` when the
/// host holds no more.
pub fn doc(surface: &str, html: &str) -> Doc {
    Doc(Cmd::new(
        Control::from([("a", "doc"), ("s", surface)]),
        html.as_bytes(),
        Quiet::ReplyOnError,
    ))
}

/// The part of a surface a placement shows, in cells from its top-left
/// corner (SPEC §5.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Window {
    /// Columns into the surface.
    pub x: u32,
    /// Rows into the surface.
    pub y: u32,
    /// Columns shown.
    pub w: u32,
    /// Rows shown.
    pub h: u32,
}

/// Where and how a surface is shown (SPEC §5.2).
///
/// `press`, `fit` and `hover` belong to the surface: `Some(true)` turns
/// one on, `Some(false)` off, and `None` leaves it as it was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Placement {
    /// The surface's width in cells, 1 to 1000.
    pub cols: u32,
    /// Its height in cells, 1 to 1000; 0 is `auto`: the host lays the
    /// document out `cols` wide, takes the rows its content needs, and says
    /// how many in the reply.
    pub rows: u32,
    /// The part of the surface shown: a full-screen program scrolling a
    /// surface out of view shows what is still in it. `None`, or all zero,
    /// shows all of it.
    pub window: Option<Window>,
    /// Stacks overlapping placements, from -1000 to 1000: greater is above.
    pub z: i32,
    /// A `press` event wherever the user presses in the placement (`p`).
    pub press: Option<bool>,
    /// A `fit` event whenever the rows the document needs at this width
    /// change (`f`). The placement keeps its size; placing it again is the
    /// program's to do.
    pub fit: Option<bool>,
    /// A `hover` event each time the element under the pointer changes,
    /// and when the pointer leaves the window (`v`).
    pub hover: Option<bool>,
    /// Leaves the cursor where it was (`C=1`); otherwise it goes to the
    /// start of the line below the placement.
    pub keep_cursor: bool,
}

impl Placement {
    /// A placement of `cols` × `rows` cells, rows 0 being `auto`, with
    /// everything else as it was.
    pub fn new(cols: u32, rows: u32) -> Placement {
        Placement {
            cols,
            rows,
            ..Placement::default()
        }
    }

    fn control(&self, surface: &str) -> Control {
        let mut c = Control::from([("a", "place"), ("s", surface)]);
        c.push("c", self.cols.to_string());
        c.push(
            "r",
            if self.rows > 0 {
                self.rows.to_string()
            } else {
                "auto".to_string()
            },
        );
        if let Some(w) = self.window.filter(|w| *w != Window::default()) {
            let whole = self.rows > 0
                && w == Window {
                    x: 0,
                    y: 0,
                    w: self.cols,
                    h: self.rows,
                };
            if !whole {
                c.push("x", w.x.to_string());
                c.push("y", w.y.to_string());
                c.push("w", w.w.to_string());
                c.push("h", w.h.to_string());
            }
        }
        if self.z != 0 {
            c.push("z", self.z.to_string());
        }
        for (k, flag) in [("p", self.press), ("f", self.fit), ("v", self.hover)] {
            if let Some(on) = flag {
                c.push(k, if on { "1" } else { "0" });
            }
        }
        if self.keep_cursor {
            c.push("C", "1");
        }
        c
    }
}

/// Places a surface at the cursor (SPEC §5.2); placing a placed surface
/// moves it. Answered on error (`q=1`): `ENOENT` when the host has no such
/// surface (it may have dropped it), `EINVAL` for a size or window out of
/// range. A reply to a placement with `auto` rows carries the rows chosen.
pub fn place(surface: &str, placement: &Placement) -> Cmd {
    Cmd::new(placement.control(surface), [], Quiet::ReplyOnError)
}

/// Places a surface with its window's top-left corner at screen cell
/// (`x`, `y`), counted from 0, and leaves the cursor where it was: the way
/// a full-screen program lays surfaces out where it draws them. It saves
/// the cursor, moves it, places with `C=1`, and restores it.
pub fn place_at(surface: &str, x: u32, y: u32, placement: &Placement) -> Cmd {
    let p = Placement {
        keep_cursor: true,
        ..*placement
    };
    let mut c = place(surface, &p);
    c.at = Some((x, y));
    c
}

/// Removes a surface's placement and keeps its document, to place it again
/// without sending it (SPEC §5.4). Deltas still apply to it.
pub fn hide(surface: &str) -> Cmd {
    Cmd::new(
        Control::from([("a", "hide"), ("s", surface)]),
        [],
        Quiet::NoReply,
    )
}

/// One change of a delta (SPEC §6.1): what to do, to which element, with
/// which name and which value. [`morph`], [`attr`], [`text`] and the rest
/// make them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Op {
    /// What it does: one of [`OPS`](crate::OPS).
    pub op: String,
    /// The id of the element, as the document has it; empty for a morph of
    /// the whole tree, and sent only when not.
    pub target: String,
    /// The name of the attribute or the custom property, for `attr`,
    /// `unattr` and `var`.
    pub key: String,
    /// The HTML, the text or the value: `None` for an op that takes none,
    /// and sent, even empty, for one that does (SDK.md §3.4.3).
    pub value: Option<String>,
}

fn op(op: &str, target: &str, key: &str, value: Option<String>) -> Op {
    Op {
        op: op.to_string(),
        target: target.to_string(),
        key: key.to_string(),
        value,
    }
}

/// Morphs an element into `html`, or with no target (`""`), each top-level
/// element of `html` into the document's element with its id (SPEC §6.2).
/// Morphing keeps what the user is doing in what stays.
pub fn morph(target: &str, html: &str) -> Op {
    op("morph", target, "", Some(html.to_string()))
}

/// Morphs the children of an element into the nodes of `html`.
pub fn inner(target: &str, html: &str) -> Op {
    op("inner", target, "", Some(html.to_string()))
}

/// Replaces an element with the nodes of `html`, without morphing.
pub fn replace(target: &str, html: &str) -> Op {
    op("replace", target, "", Some(html.to_string()))
}

/// Adds the nodes of `html` as an element's last children.
pub fn append(target: &str, html: &str) -> Op {
    op("append", target, "", Some(html.to_string()))
}

/// Adds the nodes of `html` as an element's first children.
pub fn prepend(target: &str, html: &str) -> Op {
    op("prepend", target, "", Some(html.to_string()))
}

/// Inserts the nodes of `html` before an element.
pub fn before(target: &str, html: &str) -> Op {
    op("before", target, "", Some(html.to_string()))
}

/// Inserts the nodes of `html` after an element.
pub fn after(target: &str, html: &str) -> Op {
    op("after", target, "", Some(html.to_string()))
}

/// Removes an element.
pub fn remove(target: &str) -> Op {
    op("remove", target, "", None)
}

/// Sets an attribute on an element.
pub fn attr(target: &str, name: &str, value: impl fmt::Display) -> Op {
    op("attr", target, name, Some(value.to_string()))
}

/// Removes an attribute from an element.
pub fn unattr(target: &str, name: &str) -> Op {
    op("unattr", target, name, None)
}

/// Replaces an element's children with one text node: a clock, a count.
/// Hosts make it cheap (SPEC §6.1).
pub fn text(target: &str, text: impl fmt::Display) -> Op {
    op("text", target, "", Some(text.to_string()))
}

/// Sets the custom property `--name` on an element: the cheap way to move a
/// bar every frame, with CSS that reads it (SPEC §6.1).
pub fn var(target: &str, name: &str, value: impl fmt::Display) -> Op {
    op("var", target, name, Some(value.to_string()))
}

/// Scrolls an element into view (SPEC §6.1), in each box that holds it, the
/// innermost first: with `"start"` its top to the top, with `"end"` its
/// bottom to the bottom, and with `""` by as little as brings it into view.
/// It moves no focus.
pub fn reveal_op(target: &str, where_: &str) -> Op {
    let value = matches!(where_, "start" | "end").then(|| where_.to_string());
    op("reveal", target, "", value)
}

fn takes_key(op: &str) -> bool {
    matches!(op, "attr" | "unattr" | "var")
}

fn delta_body(ops: &[Op]) -> Vec<u8> {
    let mut w = Writer::new();
    w.map(1);
    w.str("ops");
    w.array(ops.len());
    for o in ops {
        let has_t = !o.target.is_empty();
        let has_k = takes_key(&o.op);
        let v = if o.op == "reveal" {
            o.value.as_deref().filter(|v| matches!(*v, "start" | "end"))
        } else {
            o.value.as_deref()
        };
        w.map(1 + usize::from(has_t) + usize::from(has_k) + usize::from(v.is_some()));
        w.kv_str("op", &o.op);
        if has_t {
            w.kv_str("t", &o.target);
        }
        if has_k {
            w.kv_str("k", &o.key);
        }
        if let Some(v) = v {
            w.kv_str("v", v);
        }
    }
    w.finish()
}

/// Changes one surface's document (SPEC §6.1): the ops, in the order they
/// apply, which the host shows as one. A program that changes a surface in
/// one frame sends one delta (SDK.md §2.9). A delta of no ops is sent, and
/// the host refuses it (`EINVAL`). Never answered unless asked; a reply to
/// an op that failed names it by its index ([`Reply::op`](crate::Reply::op)).
///
/// Targets and values are not cleaned: they travel in the body, as the
/// document has them (SDK.md §3.4.3).
pub fn delta(surface: &str, ops: &[Op]) -> Cmd {
    Cmd::new(
        Control::from([("a", "delta"), ("s", surface)]),
        delta_body(ops),
        Quiet::NoReply,
    )
}

/// A [`delta`] of one [`text`] op.
pub fn set_text(surface: &str, target: &str, value: impl fmt::Display) -> Cmd {
    delta(surface, &[text(target, value)])
}

/// A [`delta`] of one [`var`] op.
pub fn set_var(surface: &str, target: &str, name: &str, value: impl fmt::Display) -> Cmd {
    delta(surface, &[var(target, name, value)])
}

/// A [`delta`] of one [`attr`] op.
pub fn set_attr(surface: &str, target: &str, name: &str, value: impl fmt::Display) -> Cmd {
    delta(surface, &[attr(target, name, value)])
}

/// A [`delta`] of one [`unattr`] op.
pub fn remove_attr(surface: &str, target: &str, name: &str) -> Cmd {
    delta(surface, &[unattr(target, name)])
}

/// A [`delta`] of one [`morph`] op.
pub fn morph_to(surface: &str, target: &str, html: &str) -> Cmd {
    delta(surface, &[morph(target, html)])
}

/// A [`delta`] of one [`reveal_op`]: `where_` is `"start"`, `"end"`, or
/// `""` for the nearest.
pub fn reveal(surface: &str, target: &str, where_: &str) -> Cmd {
    delta(surface, &[reveal_op(target, where_)])
}

/// A change to a control (SPEC §6.4), made against its revision: what
/// [`set_field`] sends. Each field but `rev` is absent unless given, so an
/// empty `value` or `text` and a position 0 are sent. Positions count the
/// bytes of the value's UTF-8.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FieldChange {
    /// The control's `rev` the change is made against: the last an `input`,
    /// `change` or `caret` carried ([`Event::rev`](crate::Event::rev)).
    pub rev: i64,
    /// The new value: of any control but a checkbox or a radio button.
    pub value: Option<String>,
    /// The start of the range `text` replaces, in a text field.
    pub from: Option<i64>,
    /// The end of the range `text` replaces.
    pub to: Option<i64>,
    /// Text that replaces the selection, or `from` to `to`, in a text field.
    pub text: Option<String>,
    /// The selection after the change, in a text field: its anchor, then
    /// its caret.
    pub sel: Option<(i64, i64)>,
    /// Whether a checkbox or a radio button is checked.
    pub checked: Option<bool>,
}

/// The body [`set_field`] sends: one msgpack map of `rev` and the fields
/// given, in the order of [`FieldChange`]'s.
fn field_body(c: &FieldChange) -> Vec<u8> {
    let n = 1
        + usize::from(c.value.is_some())
        + usize::from(c.from.is_some())
        + usize::from(c.to.is_some())
        + usize::from(c.text.is_some())
        + usize::from(c.sel.is_some())
        + usize::from(c.checked.is_some());
    let mut w = Writer::new();
    w.map(n);
    w.str("rev");
    w.int(c.rev);
    if let Some(v) = &c.value {
        w.kv_str("value", v);
    }
    for (k, v) in [("from", c.from), ("to", c.to)] {
        if let Some(v) = v {
            w.str(k);
            w.int(v);
        }
    }
    if let Some(t) = &c.text {
        w.kv_str("text", t);
    }
    if let Some((anchor, caret)) = c.sel {
        w.str("sel");
        w.array(2);
        w.int(anchor);
        w.int(caret);
    }
    if let Some(b) = c.checked {
        w.str("checked");
        w.bool(b);
    }
    w.finish()
}

/// Changes what the user changes in a control (SPEC §6.4): its value, the
/// text in a range, its selection, whether it is checked; focused or not.
/// The host changes it only if its `rev` is still `change.rev`, and
/// otherwise answers `ESTALE` with the control's `rev`: the user changed it
/// first, and the program decides again from the event it hears next. It
/// sends no `input` or `change`. Answered on error, so that a stale change
/// is heard. It checks nothing: the host does.
pub fn set_field(surface: &str, target: &str, change: &FieldChange) -> Cmd {
    Cmd::new(
        Control::from([("a", "field"), ("s", surface), ("t", target)]),
        field_body(change),
        Quiet::ReplyOnError,
    )
}

/// Stores a resource that documents refer to as `cid:<id>` (SPEC §7.1).
/// Sending it again replaces it, and redraws every surface that refers to
/// it.
pub fn res(id: &str, mime: &str, data: &[u8]) -> Cmd {
    Cmd::new(
        Control::from([("a", "res"), ("id", id), ("type", mime)]),
        data,
        Quiet::NoReply,
    )
}

/// Deletes a resource (SPEC §7.1).
pub fn del_res(id: &str) -> Cmd {
    Cmd::new(
        Control::from([("a", "del"), ("id", id)]),
        [],
        Quiet::NoReply,
    )
}

/// Deletes a surface and its placement (SPEC §5.4).
pub fn del(surface: &str) -> Cmd {
    Cmd::new(
        Control::from([("a", "del"), ("s", surface)]),
        [],
        Quiet::NoReply,
    )
}

/// Deletes every surface (SPEC §5.4).
pub fn del_all() -> Cmd {
    Cmd::new(Control::from([("a", "del")]), [], Quiet::NoReply)
}

/// Gives a surface up (SPEC §5.5): it stays on the screen as text does, but
/// sends no more events and never has the keyboard. A program that leaves
/// surfaces on the screen when it exits detaches them first, unless it sent
/// them [detached](Doc::detached).
pub fn detach(surface: &str) -> Cmd {
    Cmd::new(
        Control::from([("a", "detach"), ("s", surface)]),
        [],
        Quiet::NoReply,
    )
}

/// Gives a surface the keyboard (SPEC §10.1), at an element if `target` is
/// not empty; else its focused element keeps focus, or its first focusable
/// element takes it.
pub fn focus(surface: &str, target: &str) -> Cmd {
    let mut c = Control::from([("a", "focus"), ("s", surface)]);
    if !target.is_empty() {
        c.push("t", target);
    }
    Cmd::new(c, [], Quiet::NoReply)
}

/// Takes the keyboard back from a surface (SPEC §10.1). Its focused
/// control commits first, so a `change` may come before `blur`.
pub fn blur(surface: &str) -> Cmd {
    Cmd::new(
        Control::from([("a", "blur"), ("s", surface)]),
        [],
        Quiet::NoReply,
    )
}

/// Brackets commands already built in synchronized output (DEC mode 2026,
/// SPEC §6.3): the host shows the state after all of them, never one in
/// between.
pub fn sync<C: fmt::Display>(commands: impl IntoIterator<Item = C>) -> String {
    let mut s = String::from("\x1b[?2026h");
    for c in commands {
        s.push_str(&c.to_string());
    }
    s.push_str("\x1b[?2026l");
    s
}

/// Takes the edit commands named (SPEC §10.6): `select-all`, `copy`,
/// `cut`, `paste`, `undo`, `redo`. Their keys, the host's edit shortcuts
/// (the capabilities' `edit`), then reach the program as `command` events
/// in place of their bytes. Each replaces the last; none takes nothing. It
/// holds for the screen it is sent on.
pub fn edit<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Cmd {
    let take: Vec<String> = names.into_iter().map(|s| s.as_ref().to_string()).collect();
    Cmd::new(
        Control::from([("a", "edit"), ("take", take.join(",").as_str())]),
        [],
        Quiet::NoReply,
    )
}

/// An action of a program's menu, or a group of them (SPEC §10.7).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MenuItem {
    /// The key that reaches it after its group's, named as SPEC §10.4 names
    /// it; empty for an item no key reaches.
    pub key: String,
    /// What it does.
    pub label: String,
    /// The command a pick sends; empty for an item only its keys reach.
    pub name: String,
    /// Whether it does anything now.
    pub enabled: bool,
    /// The items after it, which make it a group; none make it an action.
    pub items: Vec<MenuItem>,
}

impl Default for MenuItem {
    fn default() -> MenuItem {
        MenuItem {
            key: String::new(),
            label: String::new(),
            name: String::new(),
            enabled: true,
            items: Vec::new(),
        }
    }
}

/// What a program can do (SPEC §10.7): a tree of actions with the keys that
/// reach them and the names a pick sends. It nests at most [`MENU_DEPTH`]
/// levels.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MenuTree {
    /// What the menu is for: "Files". Empty leaves it to the host.
    pub label: String,
    /// The menu's top level, in the order to show them.
    pub items: Vec<MenuItem>,
}

/// How deep a menu's items nest, the top level's being one deep (SPEC §10.7).
pub const MENU_DEPTH: usize = 8;

fn menu_items(w: &mut Writer, items: &[MenuItem]) {
    w.array(items.len());
    for i in items {
        let n = usize::from(!i.key.is_empty())
            + usize::from(!i.label.is_empty())
            + usize::from(!i.name.is_empty())
            + 1
            + usize::from(!i.items.is_empty());
        w.map(n);
        for (k, v) in [("key", &i.key), ("label", &i.label), ("name", &i.name)] {
            if !v.is_empty() {
                w.kv_str(k, v);
            }
        }
        w.str("enabled");
        w.bool(i.enabled);
        if !i.items.is_empty() {
            w.str("items");
            menu_items(w, &i.items);
        }
    }
}

/// Tells the host what the program can do now (SPEC §10.7): its menu, kept
/// until the program sends another; with `None`, the program has none. A
/// host shows it and lets the user pick from it when its capabilities have
/// the feature `menu`; on one that does not, the program shows its menu
/// itself. A pick reaches the program as a `command` event with the item's
/// name.
pub fn menu(tree: Option<&MenuTree>) -> Cmd {
    let ctl = Control::from([("a", "menu")]);
    let Some(tree) = tree else {
        return Cmd::new(ctl, [], Quiet::NoReply);
    };
    let mut w = Writer::new();
    w.map(1 + usize::from(!tree.label.is_empty()));
    if !tree.label.is_empty() {
        w.kv_str("label", &tree.label);
    }
    w.str("items");
    menu_items(&mut w, &tree.items);
    Cmd::new(ctl, w.finish(), Quiet::NoReply)
}

/// Shows the level of the program's menu that the keys typed reach (SPEC
/// §10.7): `keys` names them as SPEC §10.4 does, `"space"` and `"f"` for a
/// leader and a group; with none, the top level. A level shows until the
/// program shows another, or hides it ([`menu_hide`]).
pub fn menu_show<S: AsRef<str>>(keys: impl IntoIterator<Item = S>) -> Cmd {
    let keys: Vec<String> = keys.into_iter().map(|s| s.as_ref().to_string()).collect();
    let ctl = Control::from([("a", "menu"), ("show", "1")]);
    if keys.is_empty() {
        return Cmd::new(ctl, [], Quiet::NoReply);
    }
    let mut w = Writer::new();
    w.map(1);
    w.str("seq");
    w.array(keys.len());
    for k in &keys {
        w.str(k);
    }
    Cmd::new(ctl, w.finish(), Quiet::NoReply)
}

/// Hides the level of the menu shown (SPEC §10.7). The menu stays.
pub fn menu_hide() -> Cmd {
    Cmd::new(
        Control::from([("a", "menu"), ("show", "0")]),
        [],
        Quiet::NoReply,
    )
}

/// The way a finger drags in a [`TouchArea`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TouchDrag {
    /// Across: a horizontal slider's (`"x"`).
    #[default]
    X,
    /// Up and down (`"y"`).
    Y,
    /// Either way (`"xy"`).
    Xy,
}

impl TouchDrag {
    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            TouchDrag::X => "x",
            TouchDrag::Y => "y",
            TouchDrag::Xy => "xy",
        }
    }
}

/// Cells of the screen where a finger drags as the mouse does ([`touch`],
/// SPEC §9.6), counted from 0 at the screen's top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TouchArea {
    /// The first column it covers.
    pub c: u32,
    /// The first row it covers.
    pub r: u32,
    /// The columns it spans, at least 1.
    pub w: u32,
    /// The rows it spans, at least 1.
    pub h: u32,
    /// The way a finger drags in it.
    pub drag: TouchDrag,
}

/// How many areas a [`touch`] may name (SPEC §9.6).
pub const TOUCH_MAX: usize = 256;

/// Names the cells where a finger drags as the mouse does (SPEC §9.6), so
/// that what the program drags with the mouse in its cells, a slider, a
/// finger drags too. The areas last until the program sends others; with
/// none it leaves none. They are cells of the screen they are sent on, so
/// the program sends them again when what it drew moves, the size changes
/// or it switches screens. Keep to [`TOUCH_MAX`] areas.
pub fn touch(areas: &[TouchArea]) -> Cmd {
    let ctl = Control::from([("a", "touch")]);
    if areas.is_empty() {
        return Cmd::new(ctl, [], Quiet::NoReply);
    }
    let mut w = Writer::new();
    w.map(1);
    w.str("areas");
    w.array(areas.len());
    for a in areas {
        w.map(5);
        for (k, v) in [("c", a.c), ("r", a.r), ("w", a.w), ("h", a.h)] {
            w.str(k);
            w.int(i64::from(v));
        }
        w.kv_str("drag", a.drag.as_str());
    }
    Cmd::new(ctl, w.finish(), Quiet::NoReply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_goes_out_only_where_the_wire_does_not_imply_it() {
        assert_eq!(doc("card", "").to_string(), "\x1b]7279;a=doc:s=card\x1b\\");
        assert_eq!(
            doc("card", "").n(4).to_string(),
            "\x1b]7279;a=doc:s=card:n=4\x1b\\"
        );
        assert_eq!(
            doc("card", "").q(Quiet::NoReply).n(4).to_string(),
            "\x1b]7279;a=doc:s=card:n=4:q=2\x1b\\"
        );
        assert_eq!(hide("x").to_string(), "\x1b]7279;a=hide:s=x:q=2\x1b\\");
        assert_eq!(hide("x").n(2).to_string(), "\x1b]7279;a=hide:s=x:n=2\x1b\\");
    }

    #[test]
    fn place_at_moves_and_restores_the_cursor() {
        assert_eq!(
            place_at("c", 4, 2, &Placement::new(40, 0)).n(3).to_string(),
            "\x1b7\x1b[3;5H\x1b]7279;a=place:s=c:c=40:r=auto:C=1:n=3\x1b\\\x1b8"
        );
    }
}
