//! What the host sends: replies, events and capabilities (SDK.md §3.9).
//!
//! Their bodies are msgpack maps (SPEC §3.3), each read whole as its type:
//! one that does not decode leaves absent what it would fill, and the
//! message stands (SDK.md §2.7).

use std::fmt;

use crate::control::Control;

/// One hotty message, decoded (SDK.md §3.9): a reply or an event from the
/// host, or a command from a program (what a host, a relay or a test
/// decodes). Its control has no `m` or `o`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    /// The control, without `m` and `o`.
    pub control: Control,
    /// The payload, base64 and zlib undone.
    pub payload: Vec<u8>,
}

impl Message {
    /// A message from its control and payload.
    pub fn new(control: Control, payload: impl Into<Vec<u8>>) -> Message {
        Message {
            control,
            payload: payload.into(),
        }
    }

    /// A control key's value, or `None` when the message lacks it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.control.get(key)
    }

    /// The action, `a`: `ok`, `err`, `ev`, or a program's command.
    pub fn action(&self) -> &str {
        self.control.action()
    }

    /// The message as a reply (`a=ok` or `a=err`), or `None` when it is not one.
    pub fn reply(&self) -> Option<Reply> {
        todo!("messages: Message::reply")
    }

    /// The message as an event (`a=ev`), or `None` when it is not one.
    pub fn event(&self) -> Option<Event> {
        todo!("messages: Message::event")
    }
}

/// The host's answer to a command (SPEC §3.6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    /// `a=ok`.
    pub ok: bool,
    /// The action answered (`re`): `doc`, `place`, `q`, …; empty when absent.
    pub re: String,
    /// The command's number (`n`), when it had one.
    pub n: Option<u64>,
    /// The surface the command named (`s`), when it named one.
    pub surface: Option<String>,
    /// A placement's columns (`c`), in a reply to `place`.
    pub cols: Option<u32>,
    /// A placement's rows (`r`), in a reply to `place`: with `r=auto`, the rows the host chose.
    pub rows: Option<u32>,
    /// The index of the op at fault (`i`), from 0, in an error answering a delta (SPEC §6.1).
    pub op: Option<u32>,
    /// What went wrong, when not ok: absent when the body has none, or does not decode.
    pub code: Option<String>,
    /// The error's detail, when its body has one.
    pub detail: Option<String>,
    /// The capabilities: the body of an ok reply to `q`, when it decodes.
    pub caps: Option<Caps>,
    /// The reply as it came.
    pub message: Message,
}

impl Reply {
    /// The reply as an error, or `None` when it is ok (SDK.md §3.9's `Err`).
    pub fn err(&self) -> Option<Error> {
        if self.ok {
            return None;
        }
        Some(Error {
            code: self.code.clone().unwrap_or_default(),
            detail: self.detail.clone().unwrap_or_default(),
            re: self.re.clone(),
            surface: self.surface.clone(),
            op: self.op,
        })
    }
}

/// An error reply (SPEC §3.6) as an error: what was refused and why,
/// `hotty: delta card: ENOTARGET (go)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Error {
    /// `EINVAL`, `ENOENT`, … ([`code`](crate::code)); empty when the host gave none.
    pub code: String,
    /// The detail; empty when the host gave none.
    pub detail: String,
    /// The action answered.
    pub re: String,
    /// The surface the command named.
    pub surface: Option<String>,
    /// The index of the op at fault, in an error answering a delta.
    pub op: Option<u32>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "hotty: {}",
            if self.re.is_empty() { "?" } else { &self.re }
        )?;
        if let Some(s) = self.surface.as_deref().filter(|s| !s.is_empty()) {
            write!(f, " {s}")?;
        }
        write!(
            f,
            ": {}",
            if self.code.is_empty() {
                "error"
            } else {
                &self.code
            }
        )?;
        if let Some(i) = self.op {
            write!(f, " at op {i}")?;
        }
        if !self.detail.is_empty() {
            write!(f, " ({})", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

/// What the user did in a surface (SPEC §9). Its detail is read by its kind
/// (SDK.md §3.9): each field is `None` where the event does not carry it,
/// or its detail does not decode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    /// The surface (`s`); `None` for `command` and `caps`, which belong to none.
    pub surface: Option<String>,
    /// The kind (`e`), one of [`kind`](crate::kind) or one this SDK does not know, as it came.
    pub kind: String,
    /// The id of the element that reported (`t`); empty when absent.
    pub target: String,
    /// `value`: a `click`'s, `change`'s or `input`'s.
    pub value: Option<String>,
    /// `checked`: a `change`'s of a checkbox or radio button.
    pub checked: Option<bool>,
    /// `fields`: a `submit`'s, the pairs of a name and a value, in order (SPEC §9).
    pub fields: Option<Vec<(String, String)>>,
    /// `mods`: the modifier bits ([`MOD_SHIFT`](crate::MOD_SHIFT), …), 0 when absent.
    pub mods: u32,
    /// `href` and `url`: a `click` on a link.
    pub link: Option<Link>,
    /// `r`: a `fit`'s rows.
    pub fit_rows: Option<u32>,
    /// Where a drag's pointer is: `dragstart`, `drag`, `dragend`.
    pub drag: Option<Drag>,
    /// Where a `hover` says the pointer is.
    pub hover: Option<Hover>,
    /// `area`: the cells of the element a `click` or `press` reported.
    pub area: Option<Area>,
    /// `count`: a `click`'s place in its run of clicks (SPEC §9), 2 for the
    /// second click of a double click; 1 for a click whose detail has none;
    /// `None` for any other kind.
    pub count: Option<u32>,
    /// Which elements are in view: a `view`'s.
    pub view: Option<View>,
    pub(crate) command: Option<Command>,
    pub(crate) changed: Option<Caps>,
    /// The event as it came: an event of a kind this SDK does not know has its detail in the payload.
    pub message: Message,
}

impl Event {
    /// A `command`'s detail (SPEC §10.6, §10.7); `None` when it has no `name`.
    pub fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }

    /// A `caps` event's capabilities that changed (SPEC §4): only the fields
    /// the detail has. [`Caps::merge`] folds them in.
    pub fn changed(&self) -> Option<&Caps> {
        self.changed.as_ref()
    }
}

/// A clicked link (SPEC §9).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    /// The href, as the document has it.
    pub href: String,
    /// The href resolved against the document's base URL; `None` when it
    /// resolves to nothing outside the document.
    pub url: Option<String>,
}

/// Where a drag's pointer is (SPEC §9.1); the modifier keys held are the
/// event's `mods`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drag {
    /// The surface's column under the pointer, from 0 at its top left (not
    /// its window's); it goes on past its edges.
    pub c: i64,
    /// The surface's row under the pointer.
    pub r: i64,
    /// The step along the dragged element's width, for an element with `data-steps`.
    pub x: Option<i64>,
    /// The step down the dragged element's height.
    pub y: Option<i64>,
    /// The step along `t`'s width, for a `t` with `drop` and `data-steps`.
    pub tx: Option<i64>,
    /// The step down `t`'s height.
    pub ty: Option<i64>,
}

/// Where a `hover` event says the pointer is (SPEC §9.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hover {
    /// The surface's column under the pointer; `None` when out.
    pub c: Option<i64>,
    /// The surface's row under the pointer; `None` when out.
    pub r: Option<i64>,
    /// The pointer left the window.
    pub out: bool,
}

/// The cells an element covers, as a click, a press or a view reports them
/// (SPEC §9): from 0 at the surface's top left cell. An element partly
/// clipped or scrolled away reports its whole area, so `c` and `r` may be
/// negative or past the surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    /// The first column.
    pub c: i64,
    /// The first row.
    pub r: i64,
    /// The columns.
    pub w: i64,
    /// The rows.
    pub h: i64,
}

/// Which elements are in view, and whether the document shows its end (SPEC §9.5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// Each element with `data-on~=view` and an id that is in view, by its
    /// id, with its cells.
    pub areas: std::collections::BTreeMap<String, Area>,
    /// The document's root is scrolled as far down as it goes.
    pub at_end: bool,
}

/// An edit command, or a pick from the program's menu (SPEC §10.6, §10.7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    /// `copy`, `cut`, `paste`, `undo`, `redo` or `select-all`, or a menu item's name.
    pub name: String,
    /// The key that sent it, as SPEC §10.4 names it; empty from a menu.
    pub key: String,
    /// A paste's text; empty when it has none ([`Command::has_text`]).
    pub text: String,
    /// Whether `text` is there.
    pub has_text: bool,
}

/// A cell's size in CSS pixels (SPEC §5.3).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellCss {
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// What a host says about itself in its reply to a query (SPEC §4), and,
/// with only the fields that changed, in a `caps` event
/// ([`Event::changed`]). Each field is `None` when the host did not send it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caps {
    /// The protocol version the host answers in: `"0.4"`, for a host this SDK uses.
    pub v: Option<String>,
    /// A cell's size in CSS pixels (`cellcss`).
    pub cell_css: Option<CellCss>,
    /// Device pixels per CSS pixel: a hint for the resolution of images.
    pub scale: Option<f64>,
    /// The terminal's colour scheme: `"dark"` or `"light"`.
    pub scheme: Option<String>,
    /// The modifier `mod` stands for in bindings, `"ctrl"` or `"super"`
    /// (SPEC §10.4): the capabilities' `mod`, a Rust keyword.
    pub modifier: Option<String>,
    /// The optional features the host has ([`feature`](crate::feature)).
    pub features: Option<Vec<String>>,
    /// The host's limits, such as `surfaces` (a count), `resources` and
    /// `message` (bytes) (SPEC §13).
    pub limits: Option<std::collections::BTreeMap<String, i64>>,
    /// The host's network policy, from directive to the sources it allows (SPEC §7.2).
    pub net: Option<std::collections::BTreeMap<String, Vec<String>>>,
    /// The host's edit shortcuts, from each action to its keys, in the
    /// order it sent them (SPEC §10.2); a host that reports them takes edit
    /// commands (SPEC §10.6).
    pub edit: Option<Vec<(String, Vec<String>)>>,
    /// A name for the implementation, when it gives one.
    pub host: Option<String>,
    /// The implementation's version, with `host`.
    pub version: Option<String>,
}

impl Caps {
    /// Whether the host has an optional feature (SPEC §4): `passthrough`,
    /// `hover`, `count` or `menu`. Everything else a 0.4 host does.
    pub fn has(&self, feature: &str) -> bool {
        self.features
            .as_deref()
            .is_some_and(|f| f.iter().any(|x| x == feature))
    }

    /// Whether the terminal's colour scheme is light.
    pub fn light(&self) -> bool {
        self.scheme.as_deref() == Some("light")
    }

    /// A limit the host reported, such as `surfaces` (SPEC §13).
    pub fn limit(&self, name: &str) -> Option<i64> {
        self.limits.as_ref()?.get(name).copied()
    }

    /// These capabilities with the fields a `caps` event changed in place of
    /// their own, and without those it empties: a field that became absent
    /// comes as `{}`, `[]` or `""` (SPEC §9).
    #[must_use]
    pub fn merge(&self, changed: &Caps) -> Caps {
        let _ = changed;
        todo!("messages: Caps::merge")
    }
}
