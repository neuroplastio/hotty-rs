//! hotty sequences out of a byte stream (SDK.md §3.7).

/// One piece of what a [`Scanner`] was fed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// Bytes that are not hotty's, unchanged.
    Pass(Vec<u8>),
    /// One complete hotty sequence, from `ESC ] 7279 ;` to its terminator.
    Osc(Vec<u8>),
    /// An answer to Primary Device Attributes, `CSI ? … c`.
    Da1(Vec<u8>),
}

/// Cuts hotty sequences out of a byte stream (SDK.md §3.7).
#[derive(Debug, Default)]
pub struct Scanner {}
