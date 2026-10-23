//! hotty, HTML over the TTY (<https://github.com/neuroplastio/hotty>), from
//! a Rust program.
//!
//! A hotty host is a terminal that shows **surfaces**: small HTML documents
//! placed on rectangles of cells. A program sends documents, placements and
//! deltas as escape sequences in its ordinary output, and hears replies and
//! the user's events on its input. This crate follows the hotty
//! repository's SDK.md: the same two layers, names and defaults as the
//! other SDKs, and every SDK section of the conformance vectors.
//!
//! # The wire layer
//!
//! Pure: no I/O, no threads, no clock (SDK.md §3).
//!
//! - The builders ([`doc`], [`place`], [`set_text`], [`delta`], …) return
//!   commands that format as escape sequences, to write like any other
//!   output.
//! - A [`Scanner`] cuts hotty sequences out of the bytes a program reads, a
//!   [`Decoder`] turns them into [`Message`]s, and [`Reply`], [`Event`] and
//!   [`Caps`] read them.
//! - A [`Detector`] decides whether the terminal is a host, with the time
//!   passed in.
//! - [`parse_key`], [`decode_keys`] and [`Keymap`] say what a text field
//!   does with a key (SPEC §10.2, §10.4); [`TERMINAL_KEYS`] is the SDK's
//!   keymap, and a [`Field`] edits a value as a host does.
//!
//! # The SDK layer
//!
//! - [`link::Link`] is the program's side of the conversation, sans I/O: it is
//!   handed what the terminal sends and the time, and routes replies to
//!   what waits for them (SDK.md §4.1, §4.2).
//! - [`Terminal`] is a [`link::Link`] on the process's terminal, for a program
//!   that prints documents and exits, asks a question, or streams a chart
//!   (SDK.md §4.1). Feature `term`, Unix.
//! - [`Session`] keeps a full-screen program's surfaces on screen as its
//!   frames change, for a UI framework's adapter (SDK.md §4.3). The
//!   `hotty-ratatui` crate is one.
//!
//! # The host's side
//!
//! [`host`] is what a host or a relay reads of a program's output and
//! writes back: the output split, a program's commands read and checked,
//! and replies and events written.
//!
//! # Names
//!
//! The canonical names of SDK.md, in Rust's case. Where a name is a Rust
//! keyword or clashes with the prelude, this crate picks the nearest word:
//! - `Mod` (the capabilities' and a Platform's) is `modifier`;
//! - `Do` and `Type` of a [`Field`] are [`Field::do_action`] and
//!   [`Field::type_text`];
//! - the builders' result is a [`Cmd`], since [`Command`] is a `command`
//!   event's detail;
//! - [`Link`] is a clicked link, as in hotty-go, and the SDK layer's link
//!   is [`link::Link`].
#![cfg_attr(docsrs, feature(doc_cfg))]

mod base64;
mod commands;
mod control;
mod decoder;
mod detector;
mod encode;
pub mod field;
pub mod host;
pub mod keys;
pub mod link;
mod messages;
mod msgpack;
mod names;
mod scanner;
pub mod session;
#[cfg(all(unix, feature = "term"))]
pub mod term;

pub use commands::*;
pub use control::Control;
pub use decoder::{Decoder, Fed};
pub use detector::{DETECT_AFTER_DA1, DETECT_AFTER_REPLY, DETECT_TIMEOUT, Detector, State};
pub use encode::{PREFIX, ST, clean_value, encode, encode_plain};
pub use field::Field;
pub use keys::{
    INSERT, Keymap, Mod, Platform, TERMINAL_KEYS, decode_keys, default_edit, key_text, parse_key,
    parse_keymap, platform_of, read_key, resolve,
};
pub use messages::*;
pub use names::{surface_name, valid_name};
pub use scanner::{SCAN_MAX, Scanner, Segment};
pub use session::Session;
#[cfg(all(unix, feature = "term"))]
pub use term::Terminal;

/// The OSC number: ASCII `H` `O`, for hotty (SPEC §3.1).
pub const NUMBER: &str = "7279";
/// The most base64 bytes in one sequence (SPEC §3.4).
pub const CHUNK: usize = 4096;
/// The most columns or rows of a surface (SPEC §5.2).
pub const MAX_SIZE: u32 = 1000;
/// The longest surface name (SPEC §5).
pub const MAX_NAME: usize = 64;
/// The protocol version this SDK speaks, which its query lists (SPEC §4).
pub const VERSION: &str = "0.5";
/// The most bytes a [`Decoder`] decodes one message to unless its `max`
/// says otherwise (SDK.md §3.6): 16 MiB.
pub const DEFAULT_MAX: usize = 16 << 20;

/// Scroll the document vertically (SPEC §5.1): a bit of [`Doc::scroll`].
pub const SCROLL_VERTICAL: u32 = 1;
/// Scroll the document horizontally (SPEC §5.1): a bit of [`Doc::scroll`].
pub const SCROLL_HORIZONTAL: u32 = 2;

/// Shift was held: a bit of an event's `mods` (SPEC §9).
pub const MOD_SHIFT: u32 = 1;
/// Alt was held.
pub const MOD_ALT: u32 = 2;
/// Ctrl was held.
pub const MOD_CTRL: u32 = 4;
/// Super was held.
pub const MOD_SUPER: u32 = 8;

/// The event kinds (SPEC §9). An event of a kind not listed is an event all
/// the same, its kind as it came (SDK.md §2.7).
pub mod kind {
    /// A button, link, summary or `data-on~=click` element was activated.
    pub const CLICK: &str = "click";
    /// A control's value was committed, or a box toggled.
    pub const CHANGE: &str = "change";
    /// An edit of a control with `data-on~=input`.
    pub const INPUT: &str = "input";
    /// The user moved the selection of a text field with `data-on~=caret`,
    /// and no `input` says so (SPEC §9).
    pub const CARET: &str = "caret";
    /// A form was submitted; the detail is its fields.
    pub const SUBMIT: &str = "submit";
    /// A press anywhere in a placement made with `press`.
    pub const PRESS: &str = "press";
    /// The user focused an element; the target names it (SPEC §10.1).
    pub const FOCUS: &str = "focus";
    /// The surface gave the keyboard back.
    pub const BLUR: &str = "blur";
    /// The rows the document needs changed, on a placement made with `fit`.
    pub const FIT: &str = "fit";
    /// The element under the pointer changed, or the pointer left (SPEC §9.4).
    pub const HOVER: &str = "hover";
    /// Which elements with `data-on~=view` are in view changed (SPEC §9.5).
    pub const VIEW: &str = "view";
    /// The primary button pressed on a draggable element (SPEC §9.1).
    pub const DRAG_START: &str = "dragstart";
    /// The element under the pointer changed during a drag.
    pub const DRAG: &str = "drag";
    /// The button released, or the drag cut short.
    pub const DRAG_END: &str = "dragend";
    /// An edit command, by its key or a menu, or a pick from the program's
    /// menu (SPEC §10.6, §10.7). It belongs to no surface.
    pub const COMMAND: &str = "command";
    /// A capability changed (SPEC §4). It belongs to no surface.
    pub const CAPS: &str = "caps";
    /// Every kind this SDK knows, in SPEC §9's order.
    pub const ALL: [&str; 16] = [
        CLICK, CHANGE, INPUT, CARET, SUBMIT, PRESS, FOCUS, BLUR, FIT, HOVER, VIEW, DRAG_START,
        DRAG, DRAG_END, COMMAND, CAPS,
    ];
}

/// The optional features a host may have (SPEC §4), which [`Caps::has`]
/// asks about.
pub mod feature {
    /// The pointer passes through the parts of a surface that take none (SPEC §9.3).
    pub const PASSTHROUGH: &str = "passthrough";
    /// It says where the pointer is, with hover events (SPEC §9.4).
    pub const HOVER: &str = "hover";
    /// It says that a click repeats one before it (SPEC §9).
    pub const COUNT: &str = "count";
    /// It keeps a program's menu, shows its levels and lets the user pick from it (SPEC §10.7).
    pub const MENU: &str = "menu";
    /// Every feature this SDK knows.
    pub const ALL: [&str; 4] = [PASSTHROUGH, HOVER, COUNT, MENU];
}

/// The error codes a host replies (SPEC §3.6).
pub mod code {
    /// An unknown action or op, a bad name, a missing key, a value out of range.
    pub const EINVAL: &str = "EINVAL";
    /// No surface of that name.
    pub const ENOENT: &str = "ENOENT";
    /// No element with that id; the detail is the id, or ids comma-separated.
    pub const ENOTARGET: &str = "ENOTARGET";
    /// The surface is detached (SPEC §5.5).
    pub const EDETACHED: &str = "EDETACHED";
    /// Over one of the host's limits (SPEC §13).
    pub const EQUOTA: &str = "EQUOTA";
    /// The host gave up on work over its time budget.
    pub const EBUDGET: &str = "EBUDGET";
    /// Nothing can lay the document out yet: a multiplexer with no terminal
    /// attached; the rows come as `fit` (SPEC §5.2).
    pub const ENOTERM: &str = "ENOTERM";
    /// An `a=field` made against a revision the control has passed, or
    /// while an input method composes in it; the detail is the control's
    /// revision, in decimal (SPEC §6.4).
    pub const ESTALE: &str = "ESTALE";
    /// A query lists no version the host speaks; the detail is those it speaks (SPEC §4).
    pub const EVERSION: &str = "EVERSION";
    /// Every code this SDK knows.
    pub const ALL: [&str; 9] = [
        EINVAL, ENOENT, ENOTARGET, EDETACHED, EQUOTA, EBUDGET, ENOTERM, ESTALE, EVERSION,
    ];
}

/// The delta ops (SPEC §6.1), which [`Op::op`] names.
pub const OPS: [&str; 13] = [
    "morph", "inner", "replace", "append", "prepend", "before", "after", "remove", "attr",
    "unattr", "text", "var", "reveal",
];
