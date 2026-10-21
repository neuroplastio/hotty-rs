//! What the host sends: replies, events and capabilities (SDK.md §3.9).
//!
//! Their bodies are msgpack maps (SPEC §3.3), each read whole as its type:
//! one that does not decode leaves absent what it would fill, and the
//! message stands (SDK.md §2.7).

use std::collections::BTreeMap;
use std::fmt;

use crate::control::Control;
use crate::msgpack::Value;

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

    /// The payload as UTF-8 text, a character that is not UTF-8 replaced:
    /// a document's markup.
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.payload)
    }

    /// The message as a reply (`a=ok` or `a=err`), or `None` when it is not one.
    pub fn reply(&self) -> Option<Reply> {
        let ok = match self.action() {
            "ok" => true,
            "err" => false,
            _ => return None,
        };
        let re = self.get("re").unwrap_or_default().to_owned();
        let mut r = Reply {
            ok,
            n: number(self.get("n")),
            surface: self.get("s").map(str::to_owned),
            cols: number(self.get("c")),
            rows: number(self.get("r")),
            op: number(self.get("i")),
            message: self.clone(),
            ..Reply::default()
        };
        if ok {
            if re == "q" {
                r.caps = body::read(&self.payload, &body::CAPS).map(|b| body::caps(&b, false));
            }
        } else if let Some(b) = body::read(&self.payload, &body::ERROR) {
            r.code = body::str(&b, "code");
            r.detail = body::str(&b, "detail");
        }
        r.re = re;
        Some(r)
    }

    /// The message as an event (`a=ev`), or `None` when it is not one. Its
    /// detail is read by its kind (SDK.md §3.9): a field named `value` in a
    /// form's `submit` is in `fields`, and nowhere else.
    pub fn event(&self) -> Option<Event> {
        if self.action() != "ev" {
            return None;
        }
        let kind = self.get("e").unwrap_or_default().to_owned();
        let d = body::detail(&kind).and_then(|t| body::read(&self.payload, t));
        let d = d.as_ref();
        let k = kind.as_str();
        let str_of = |key| d.and_then(|d| body::str(d, key));
        let int_of = |key| d.and_then(|d| d.get(key)).and_then(Value::as_int);
        let mut e = Event {
            surface: self.get("s").map(str::to_owned),
            target: self.get("t").unwrap_or_default().to_owned(),
            message: self.clone(),
            ..Event::default()
        };
        if matches!(k, "click" | "change" | "input") {
            e.value = str_of("value");
        }
        if k == "change" {
            e.checked = d.and_then(|d| d.get("checked")).and_then(Value::as_bool);
        }
        if k == "submit" {
            e.fields = d.and_then(|d| body::fields(d.get("fields")?));
        }
        if matches!(k, "click" | "press" | "dragstart" | "drag" | "dragend") {
            e.mods = int_of("mods")
                .and_then(|m| u32::try_from(m).ok())
                .unwrap_or(0);
        }
        if matches!(k, "click" | "press") {
            e.area = d.and_then(|d| body::area(d.get("area")?));
        }
        match k {
            "click" => {
                e.link = str_of("href").map(|href| Link {
                    href,
                    url: str_of("url"),
                });
                e.count = Some(
                    int_of("count")
                        .filter(|&n| n > 1)
                        .and_then(|n| u32::try_from(n).ok())
                        .unwrap_or(1),
                );
            }
            "fit" => {
                e.fit_rows = int_of("r")
                    .filter(|&r| r >= 1)
                    .and_then(|r| u32::try_from(r).ok())
            }
            "dragstart" | "drag" | "dragend" => {
                if let (Some(c), Some(r)) = (int_of("c"), int_of("r")) {
                    e.drag = Some(Drag {
                        c,
                        r,
                        x: int_of("x"),
                        y: int_of("y"),
                        tx: int_of("tx"),
                        ty: int_of("ty"),
                    });
                }
            }
            "hover" => {
                let out = d.and_then(|d| d.get("out")).and_then(Value::as_bool) == Some(true);
                e.hover = if out {
                    Some(Hover {
                        c: None,
                        r: None,
                        out: true,
                    })
                } else if let (Some(c), Some(r)) = (int_of("c"), int_of("r")) {
                    Some(Hover {
                        c: Some(c),
                        r: Some(r),
                        out: false,
                    })
                } else {
                    None
                };
            }
            "view" => e.view = d.map(body::view),
            "command" => {
                e.command = str_of("name").map(|name| {
                    let text = str_of("text");
                    Command {
                        name,
                        key: str_of("key").unwrap_or_default(),
                        has_text: text.is_some(),
                        text: text.unwrap_or_default(),
                    }
                })
            }
            "caps" => e.changed = d.map(|d| body::caps(d, true)),
            _ => {}
        }
        e.kind = kind;
        Some(e)
    }
}

/// A control value as a number: `None` when it is not one, digits only.
fn number<T: std::str::FromStr>(v: Option<&str>) -> Option<T> {
    let v = v?;
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse().ok()
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
    /// A `command`'s detail (SPEC §10.6, §10.7), as [`Event::command`] reads it.
    pub command: Option<Command>,
    /// A `caps` event's capabilities that changed, as [`Event::changed`] reads them.
    pub changed: Option<Caps>,
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
    pub areas: BTreeMap<String, Area>,
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
    pub limits: Option<BTreeMap<String, i64>>,
    /// The host's network policy, from directive to the sources it allows (SPEC §7.2).
    pub net: Option<BTreeMap<String, Vec<String>>>,
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
    ///
    /// In a `caps` event's [`changed`](Event::changed) capabilities, a
    /// `cellcss` the host emptied, or sent without its `w` or `h`, is a
    /// zero [`CellCss`], which merging makes absent.
    #[must_use]
    pub fn merge(&self, changed: &Caps) -> Caps {
        fn put<T: Clone>(to: &mut Option<T>, from: &Option<T>, empty: impl Fn(&T) -> bool) {
            if let Some(v) = from {
                *to = (!empty(v)).then(|| v.clone());
            }
        }
        let mut c = self.clone();
        put(&mut c.v, &changed.v, String::is_empty);
        put(&mut c.cell_css, &changed.cell_css, |x| {
            *x == CellCss::default()
        });
        put(&mut c.scale, &changed.scale, |_| false);
        put(&mut c.scheme, &changed.scheme, String::is_empty);
        put(&mut c.modifier, &changed.modifier, String::is_empty);
        put(&mut c.features, &changed.features, Vec::is_empty);
        put(&mut c.limits, &changed.limits, BTreeMap::is_empty);
        put(&mut c.net, &changed.net, BTreeMap::is_empty);
        put(&mut c.edit, &changed.edit, Vec::is_empty);
        put(&mut c.host, &changed.host, String::is_empty);
        put(&mut c.version, &changed.version, String::is_empty);
        c
    }

    /// The names of the fields these capabilities have, as a host's body
    /// names them (`cellcss`, `mod`): for a `caps` event's
    /// [`changed`](Event::changed) capabilities, what changed.
    pub fn fields(&self) -> Vec<&'static str> {
        [
            ("v", self.v.is_some()),
            ("cellcss", self.cell_css.is_some()),
            ("scale", self.scale.is_some()),
            ("scheme", self.scheme.is_some()),
            ("mod", self.modifier.is_some()),
            ("features", self.features.is_some()),
            ("limits", self.limits.is_some()),
            ("net", self.net.is_some()),
            ("edit", self.edit.is_some()),
            ("host", self.host.is_some()),
            ("version", self.version.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, has)| has.then_some(name))
        .collect()
    }
}

/// The types of a host's bodies (SPEC §3.3): an error's (§3.6), the
/// capabilities (§4), and each event kind's detail (§9). A body is read
/// whole as its type (SDK.md §3.9): one msgpack map that [`msgpack::read`]
/// takes, each field its type names of that type, and any other field of
/// any type.
mod body {
    use std::collections::BTreeMap;

    use super::{Area, Caps, CellCss, View};
    use crate::msgpack::{self, Value};

    /// A field's type: one of SPEC §3.3's four, an array of a type, a map
    /// by its fields' types, or a map of any names to a type.
    pub(super) enum Ty {
        Int,
        Float,
        Str,
        Bool,
        Array(&'static Ty),
        Map(&'static [(&'static str, Ty)]),
        Each(&'static Ty),
    }

    use Ty::{Array, Bool, Each, Float, Int, Map, Str};

    pub(super) const ERROR: Ty = Map(&[("code", Str), ("detail", Str)]);

    const STRS: Ty = Array(&Str);

    pub(super) const CAPS: Ty = Map(&[
        ("v", Str),
        ("cellcss", Map(&[("w", Float), ("h", Float)])),
        ("scale", Float),
        ("scheme", Str),
        ("mod", Str),
        ("features", STRS),
        ("limits", Each(&Int)),
        ("net", Each(&STRS)),
        ("edit", Each(&STRS)),
        ("host", Str),
        ("version", Str),
    ]);

    const AREA: Ty = Map(&[("c", Int), ("r", Int), ("w", Int), ("h", Int)]);

    const DRAG: Ty = Map(&[
        ("c", Int),
        ("r", Int),
        ("mods", Int),
        ("x", Int),
        ("y", Int),
        ("tx", Int),
        ("ty", Int),
    ]);

    /// The type of a kind's detail; `None` for a kind whose detail this SDK
    /// does not read.
    pub(super) fn detail(kind: &str) -> Option<&'static Ty> {
        Some(match kind {
            "click" => &Map(&[
                ("value", Str),
                ("href", Str),
                ("url", Str),
                ("area", AREA),
                ("mods", Int),
                ("count", Int),
            ]),
            "press" => &Map(&[("area", AREA), ("mods", Int)]),
            "change" => &Map(&[("checked", Bool), ("value", Str)]),
            "input" => &Map(&[("value", Str)]),
            "submit" => &Map(&[("fields", Array(&STRS))]),
            "fit" => &Map(&[("r", Int)]),
            "dragstart" | "drag" | "dragend" => &DRAG,
            "hover" => &Map(&[("c", Int), ("r", Int), ("out", Bool)]),
            "view" => &Map(&[("in", Each(&AREA)), ("end", Bool)]),
            "command" => &Map(&[("name", Str), ("key", Str), ("text", Str)]),
            "caps" => &CAPS,
            _ => return None,
        })
    }

    fn is(v: &Value, t: &Ty) -> bool {
        match (t, v) {
            (Int, Value::Int(_)) | (Float, Value::Float(_)) | (Str, Value::Str(_)) => true,
            (Bool, Value::Bool(_)) => true,
            (Array(t), Value::Array(a)) => a.iter().all(|v| is(v, t)),
            (Each(t), Value::Map(m)) => m.iter().all(|(_, v)| is(v, t)),
            (Map(fields), Value::Map(m)) => m.iter().all(|(k, v)| {
                fields
                    .iter()
                    .find(|(name, _)| name == k)
                    .is_none_or(|(_, t)| is(v, t))
            }),
            _ => false,
        }
    }

    /// The body read as its type: `None` when it does not decode.
    pub(super) fn read(payload: &[u8], t: &Ty) -> Option<Value> {
        let v = msgpack::read(payload)?;
        (matches!(v, Value::Map(_)) && is(&v, t)).then_some(v)
    }

    pub(super) fn str(v: &Value, key: &str) -> Option<String> {
        v.get(key)?.as_str().map(str::to_owned)
    }

    fn strs(v: &Value) -> Option<Vec<String>> {
        v.as_array()?
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect()
    }

    /// A submit's fields: each a pair of a name and a value, or none.
    pub(super) fn fields(v: &Value) -> Option<Vec<(String, String)>> {
        v.as_array()?
            .iter()
            .map(|p| match strs(p)?.as_slice() {
                [name, value] => Some((name.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    /// An element's cells: absent unless the map has all of c, r, w and h.
    pub(super) fn area(v: &Value) -> Option<Area> {
        let n = |k| v.get(k)?.as_int();
        Some(Area {
            c: n("c")?,
            r: n("r")?,
            w: n("w")?,
            h: n("h")?,
        })
    }

    /// A view's elements in view, leaving out one whose area lacks a field.
    pub(super) fn view(d: &Value) -> View {
        let areas = d
            .get("in")
            .and_then(Value::as_map)
            .unwrap_or_default()
            .iter()
            .filter_map(|(id, a)| Some((id.clone(), area(a)?)))
            .collect();
        View {
            areas,
            at_end: d.get("end").and_then(Value::as_bool) == Some(true),
        }
    }

    /// The capabilities of a body read as [`CAPS`]. A `caps` event's
    /// (`changed`) keep a `cellcss` that lacks `w` or `h` as a zero one,
    /// so that merging empties it.
    pub(super) fn caps(b: &Value, changed: bool) -> Caps {
        let cell_css = b.get("cellcss").and_then(|c| {
            let f = |k| c.get(k)?.as_float();
            match (f("w"), f("h")) {
                (Some(w), Some(h)) => Some(CellCss { w, h }),
                _ if changed => Some(CellCss::default()),
                _ => None,
            }
        });
        let each = |k| -> Option<Vec<(String, Value)>> { Some(b.get(k)?.as_map()?.to_vec()) };
        Caps {
            v: str(b, "v"),
            cell_css,
            scale: b.get("scale").and_then(Value::as_float),
            scheme: str(b, "scheme"),
            modifier: str(b, "mod"),
            features: b.get("features").and_then(strs),
            limits: each("limits").map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_int()?)))
                    .collect::<BTreeMap<_, _>>()
            }),
            net: each("net").map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), strs(v)?)))
                    .collect::<BTreeMap<_, _>>()
            }),
            edit: each("edit").map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), strs(v)?)))
                    .collect()
            }),
            host: str(b, "host"),
            version: str(b, "version"),
        }
    }
}
