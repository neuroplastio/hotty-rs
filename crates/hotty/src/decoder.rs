//! OSC sequences into messages (SDK.md §3.6).

use crate::messages::Message;

/// What [`Decoder::feed`] made of one sequence.
#[derive(Clone, Debug, PartialEq)]
pub enum Fed {
    /// Another OSC, or another number: the caller handles it.
    NotHotty,
    /// A chunk of a message that is not complete yet.
    Partial,
    /// A whole message.
    Complete(Message),
    /// A malformed message (SPEC §3.7), dropped.
    Invalid,
}

/// Turns OSC sequences into messages, one at a time, joining chunked ones
/// (SDK.md §3.6). A new Decoder is ready to use.
#[derive(Debug, Default)]
pub struct Decoder {
    /// The most bytes one message decodes to, after base64 and zlib; 0 is
    /// [`DEFAULT_MAX`](crate::DEFAULT_MAX).
    pub max: usize,
}

impl Decoder {
    /// A Decoder with the default `max`.
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Takes one OSC sequence, `ESC ] …`, with or without its terminator.
    pub fn feed(&mut self, seq: &[u8]) -> Fed {
        let _ = seq;
        todo!("decoder: feed")
    }

    /// The malformed messages dropped so far.
    pub fn invalid(&self) -> usize {
        todo!("decoder: invalid")
    }
}
