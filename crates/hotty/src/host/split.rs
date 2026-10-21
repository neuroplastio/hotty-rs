//! A program's output, split for a host.

use crate::decoder::{Decoder, Fed};
use crate::messages::Message;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const PREFIX: &[u8] = b"\x1b]7279;";

/// The control sequences a host acts on: everything else in a program's
/// output passes as [`Output::Bytes`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Seq {
    /// `CSI ? Pm h` (`set`) or `CSI ? Pm l`: private modes, such as 1049
    /// (the alternate screen, SPEC §5.4), 2026 (synchronized output, SPEC
    /// §6.3) and the mouse's (SPEC §9.6).
    PrivateMode {
        /// `h` rather than `l`.
        set: bool,
        /// The modes, in order.
        modes: Vec<u16>,
    },
    /// `CSI Ps J`: erase in display.
    EraseDisplay(u16),
    /// `ESC c`: a full reset, which deletes every surface (SPEC §5.4).
    Reset,
}

/// A piece of a program's output, as a [`Splitter`] finds it, in order.
#[derive(Debug, PartialEq)]
pub enum Output<'a> {
    /// Bytes to pass on to the terminal as they are.
    Bytes(&'a [u8]),
    /// A control sequence a host may act on, and its bytes, which the host
    /// passes on or withholds itself.
    Seq(Seq, &'a [u8]),
    /// A hotty command, its chunks joined and its payload decoded.
    Command(Message),
    /// A hotty command dropped as malformed (SPEC §3.7), and why: nothing
    /// of it passes on.
    Invalid(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    OscNum,
    OscOther,
    OscOtherEsc,
    StrOther,
    StrOtherEsc,
    Csi,
    Ours,
    OursEsc,
    Discard,
    DiscardEsc,
}

/// Splits a program's output for a host or a relay, however its reads
/// split it: the bytes to pass on, the control sequences a host acts on
/// ([`Seq`]), and the hotty commands, decoded by a [`Decoder`] (SPEC §3).
///
/// It holds back only what may still become a sequence it must see whole:
/// an escape sequence's first bytes, a CSI up to its final byte, and a
/// hotty sequence. Other strings (OSC, DCS, APC, PM, SOS) pass as they come,
/// and an `ESC c` inside one is no reset.
#[derive(Debug)]
pub struct Splitter {
    state: State,
    pending: Vec<u8>,
    ours: Vec<u8>,
    decoder: Decoder,
    // A chunked command one of whose sequences was too large to hold: its
    // further chunks are dropped, unreported, up to its last.
    dropping: bool,
}

impl Default for Splitter {
    fn default() -> Splitter {
        Splitter::new()
    }
}

impl Splitter {
    /// A splitter whose Decoder has the default `max`.
    pub fn new() -> Splitter {
        Splitter::with_max(0)
    }

    /// A splitter that decodes no command to more than `max` bytes; 0 is
    /// [`DEFAULT_MAX`](crate::DEFAULT_MAX). A host reports it as
    /// `limits.message` (SPEC §13).
    pub fn with_max(max: usize) -> Splitter {
        Splitter {
            state: State::Ground,
            pending: Vec::with_capacity(32),
            ours: Vec::new(),
            decoder: Decoder::with_max(max),
            dropping: false,
        }
    }

    /// The Decoder it decodes hotty's sequences with: its `max`, and its
    /// count of malformed messages.
    pub fn decoder(&self) -> &Decoder {
        &self.decoder
    }

    /// Whether it holds bytes back: the start of a sequence, or a hotty
    /// sequence in progress.
    pub fn is_holding(&self) -> bool {
        !self.pending.is_empty() || matches!(self.state, State::Ours | State::OursEsc)
    }

    // The longest hotty sequence it keeps: what the Decoder's max allows,
    // and a control's worth beside.
    fn seq_max(&self) -> usize {
        let max = if self.decoder.max > 0 {
            self.decoder.max
        } else {
            crate::DEFAULT_MAX
        };
        (max + max / 256 + 64).div_ceil(3) * 4 + 4096
    }

    /// Takes the next bytes of the program's output and hands `sink` what
    /// they complete, in order.
    pub fn feed(&mut self, input: &[u8], sink: &mut dyn FnMut(Output<'_>)) {
        // `run` starts a stretch of input that passes as it is.
        let mut run = 0usize;
        let mut i = 0usize;
        while i < input.len() {
            let b = input[i];
            match self.state {
                State::Ground => {
                    if b == ESC {
                        flush(sink, &input[run..i]);
                        self.pending.push(b);
                        self.state = State::Esc;
                        run = i + 1;
                    }
                }
                State::Esc => {
                    self.pending.push(b);
                    run = i + 1;
                    match b {
                        b']' => self.state = State::OscNum,
                        b'[' => self.state = State::Csi,
                        b'P' | b'_' | b'^' | b'X' => {
                            self.flush_pending(sink);
                            self.state = State::StrOther;
                        }
                        b'c' => {
                            sink(Output::Seq(Seq::Reset, &self.pending));
                            self.pending.clear();
                            self.state = State::Ground;
                        }
                        ESC => {
                            // ESC ESC: the first is inert here; the second
                            // may begin a sequence.
                            let last = self.pending.len() - 1;
                            sink(Output::Bytes(&self.pending[..last]));
                            self.pending.clear();
                            self.pending.push(ESC);
                        }
                        _ => {
                            self.flush_pending(sink);
                            self.state = State::Ground;
                        }
                    }
                }
                State::OscNum => {
                    if b.is_ascii_digit() && self.pending.len() < 12 {
                        self.pending.push(b);
                        run = i + 1;
                    } else if b == b';' && self.pending[2..] == PREFIX[2..PREFIX.len() - 1] {
                        self.pending.clear();
                        self.ours.clear();
                        self.state = State::Ours;
                        run = i + 1;
                    } else {
                        // Someone else's OSC: what was held passes, and so
                        // does the rest of it.
                        self.flush_pending(sink);
                        run = i;
                        self.state = State::OscOther;
                        continue;
                    }
                }
                State::OscOther => match b {
                    BEL => self.state = State::Ground,
                    ESC => self.state = State::OscOtherEsc,
                    _ => {}
                },
                State::OscOtherEsc => {
                    self.state = match b {
                        b'\\' => State::Ground,
                        ESC => State::OscOtherEsc,
                        _ => State::OscOther,
                    }
                }
                State::StrOther => {
                    if b == ESC {
                        self.state = State::StrOtherEsc;
                    }
                }
                State::StrOtherEsc => {
                    self.state = match b {
                        b'\\' => State::Ground,
                        ESC => State::StrOtherEsc,
                        _ => State::StrOther,
                    }
                }
                State::Csi => {
                    self.pending.push(b);
                    run = i + 1;
                    if (0x40..=0x7e).contains(&b) {
                        self.finish_csi(sink);
                        self.state = State::Ground;
                    } else if self.pending.len() > 64 {
                        self.flush_pending(sink);
                        self.state = State::Ground;
                    }
                }
                State::Ours => {
                    run = i + 1;
                    match b {
                        BEL => {
                            self.finish(sink);
                            self.state = State::Ground;
                        }
                        ESC => self.state = State::OursEsc,
                        _ => {
                            self.ours.push(b);
                            if self.ours.len() > self.seq_max() {
                                // Longer than any command the Decoder takes:
                                // dropped up to its end, and a chunked
                                // command it belongs to with it.
                                sink(Output::Invalid("hotty command too large"));
                                let ctl = self.ours.split(|&b| b == b';').next().unwrap_or(&[]);
                                self.dropping = ctl.split(|&b| b == b':').any(|kv| kv == b"m=1")
                                    || is_continuation(ctl);
                                self.ours = Vec::new();
                                self.decoder.abort();
                                self.state = State::Discard;
                            }
                        }
                    }
                }
                State::OursEsc => {
                    if b == b'\\' {
                        run = i + 1;
                        self.finish(sink);
                        self.state = State::Ground;
                    } else {
                        // An ESC that does not begin ST ends the sequence
                        // unfinished (SDK.md §3.7); it begins what follows.
                        sink(Output::Invalid("hotty command cut short by ESC"));
                        self.ours.clear();
                        self.pending.push(ESC);
                        self.state = State::Esc;
                        run = i;
                        continue;
                    }
                }
                State::Discard => {
                    run = i + 1;
                    match b {
                        BEL => self.state = State::Ground,
                        ESC => self.state = State::DiscardEsc,
                        _ => {}
                    }
                }
                State::DiscardEsc => {
                    run = i + 1;
                    self.state = if b == b'\\' {
                        State::Ground
                    } else {
                        State::Discard
                    };
                }
            }
            i += 1;
        }
        flush(sink, &input[run..]);
    }

    fn flush_pending(&mut self, sink: &mut dyn FnMut(Output<'_>)) {
        if !self.pending.is_empty() {
            sink(Output::Bytes(&self.pending));
            self.pending.clear();
        }
    }

    fn finish_csi(&mut self, sink: &mut dyn FnMut(Output<'_>)) {
        let body = &self.pending[2..];
        let final_byte = *body.last().unwrap_or(&0);
        let params = &body[..body.len().saturating_sub(1)];
        let seq = if let Some(rest) = params.strip_prefix(b"?") {
            (matches!(final_byte, b'h' | b'l')
                && rest.iter().all(|c| c.is_ascii_digit() || *c == b';'))
            .then(|| Seq::PrivateMode {
                set: final_byte == b'h',
                modes: numbers(rest),
            })
        } else if final_byte == b'J' && params.iter().all(u8::is_ascii_digit) {
            Some(Seq::EraseDisplay(
                numbers(params).first().copied().unwrap_or(0),
            ))
        } else {
            None
        };
        match seq {
            Some(seq) => sink(Output::Seq(seq, &self.pending)),
            None => sink(Output::Bytes(&self.pending)),
        }
        self.pending.clear();
    }

    // A whole hotty sequence: to the Decoder, which joins its chunks and
    // decodes it. Each message it drops is reported once.
    fn finish(&mut self, sink: &mut dyn FnMut(Output<'_>)) {
        if self.dropping {
            let ctl = self.ours.split(|&b| b == b';').next().unwrap_or(&[]);
            if is_continuation(ctl) {
                self.dropping = ctl == b"m=1";
                self.ours.clear();
                return;
            }
            self.dropping = false;
        }
        let mut seq = Vec::with_capacity(PREFIX.len() + self.ours.len());
        seq.extend_from_slice(PREFIX);
        seq.append(&mut self.ours);
        let before = self.decoder.invalid();
        let fed = self.decoder.feed(&seq);
        let dropped = self.decoder.invalid() - before;
        if dropped == 2 {
            sink(Output::Invalid(crate::decoder::INTERRUPTED));
        }
        if dropped >= 1 {
            sink(Output::Invalid(self.decoder.why()));
        }
        if let Fed::Complete(m) = fed {
            sink(Output::Command(m));
        }
    }
}

// A chunk's control that carries only `m` (SPEC §3.4).
fn is_continuation(ctl: &[u8]) -> bool {
    ctl == b"m=0" || ctl == b"m=1"
}

fn flush(sink: &mut dyn FnMut(Output<'_>), bytes: &[u8]) {
    if !bytes.is_empty() {
        sink(Output::Bytes(bytes));
    }
}

fn numbers(bytes: &[u8]) -> Vec<u16> {
    bytes
        .split(|&c| c == b';')
        .map(|p| {
            std::str::from_utf8(p)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        })
        .collect()
}
