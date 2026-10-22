//! The host's side of the wire: what a host, or a relay standing between
//! programs and a host (SPEC §2, SDK.md §4.5), reads of a program's output
//! and writes back.
//!
//! - [`Splitter`] cuts a program's output into the bytes to pass on, the
//!   control sequences a host acts on, and hotty's commands, decoded.
//! - [`delta_of`], [`field_of`], [`menu_of`], [`menu_show_of`],
//!   [`touch_of`] and [`placement_of`] read a command as SPEC has a host
//!   read it, and refuse one it would refuse with the `EINVAL` to answer;
//!   [`control_kind`] and [`field_fits`] say whether an `a=field` fits its
//!   target.
//! - [`reply_ok`], [`reply_err`], [`reply_query`] and [`encode_event`]
//!   write what a host sends, each body typed as SPEC §3.3 types it.
//!
//! ```
//! use hotty::host::{Output, Splitter, delta_of, reply_err};
//!
//! let mut out = Vec::new();
//! let mut replies = Vec::new();
//! let program = format!("hi{}", hotty::set_text("card", "t", "12:00").n(3));
//! Splitter::new().feed(program.as_bytes(), &mut |o| match o {
//!     Output::Bytes(b) | Output::Seq(_, b) => out.extend_from_slice(b),
//!     Output::Command(cmd) => match delta_of(&cmd) {
//!         Ok(ops) => assert_eq!(ops[0].value.as_deref(), Some("12:00")),
//!         Err(e) => replies.push(reply_err(&cmd, &e)),
//!     },
//!     Output::Invalid(why) => eprintln!("dropped: {why}"),
//! });
//! assert_eq!(out, b"hi");
//! ```

mod read;
mod split;
mod write;

pub use read::{
    ControlKind, control_kind, cursor_below, delta_of, field_fits, field_of, menu_of, menu_show_of,
    placement_of, quiet_of, speaks, touch_of,
};
pub use split::{Output, Seq, Splitter};
pub use write::{
    caps_body, encode_event, error_body, reply_caps, reply_err, reply_ok, reply_query,
};
