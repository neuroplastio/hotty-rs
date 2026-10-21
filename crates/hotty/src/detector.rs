//! Whether the terminal is a host (SDK.md §3.8).

/// How long detection waits for any answer, in milliseconds.
pub const DETECT_TIMEOUT: u64 = 1500;
/// The grace a DA1 before any reply opens, in milliseconds.
pub const DETECT_AFTER_DA1: u64 = 150;
/// How long detection waits for the DA1 behind a reply, in milliseconds.
pub const DETECT_AFTER_REPLY: u64 = 300;

/// What a [`Detector`] found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Not known yet.
    #[default]
    Detecting,
    /// A host the program can use.
    Native,
    /// Not a host, or not one of this version.
    Text,
}

/// Decides whether the terminal is a host (SDK.md §3.8).
#[derive(Debug, Default)]
pub struct Detector {}
