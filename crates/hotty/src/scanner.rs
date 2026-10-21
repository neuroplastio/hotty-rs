//! hotty sequences out of a byte stream, however the reads split it
//! (SDK.md §3.7).

/// One piece of what a [`Scanner`] was fed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// Bytes that are not hotty's, unchanged: keys, mouse reports, other
    /// OSCs, other answers.
    Pass(Vec<u8>),
    /// One complete hotty sequence, from `ESC ] 7279 ;` to its terminator
    /// (ST or BEL), ready for a [`Decoder`](crate::Decoder).
    Osc(Vec<u8>),
    /// An answer to Primary Device Attributes, `CSI ? <digits and ;> c`,
    /// from a Scanner that looks for them ([`Scanner::da1`]).
    Da1(Vec<u8>),
}

/// The longest hotty sequence a Scanner keeps, its terminator not counted:
/// 64 KiB. No host sends a longer one, since chunks are at most 4096 bytes
/// of base64.
pub const SCAN_MAX: usize = 64 << 10;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const PREFIX: &[u8] = b"\x1b]7279;";
const DA1_PREFIX: &[u8] = b"\x1b[?";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Ground,
    Osc,
    Discard,
}

/// Cuts hotty sequences out of a byte stream (SDK.md §3.7). An SDK that
/// reads a raw terminal uses one; so does a relay, on a program's output.
///
/// It holds only what may still become a segment of its own: the start of
/// `ESC ] 7279 ;` (or of a DA1 answer, when it looks for them), and a hotty
/// sequence in progress. Everything else goes out in the same
/// [`feed`](Scanner::feed) that brought it, so the segments do not depend on
/// how the stream was split, and a key is never kept waiting for the next
/// read.
#[derive(Clone, Debug, Default)]
pub struct Scanner {
    da1: bool,
    invalid: usize,
    mode: Mode,
    held: Vec<u8>,
    esc: bool,
    out: Vec<Segment>,
    pass: Vec<u8>,
}

impl Scanner {
    /// A Scanner that passes DA1 answers as they are.
    pub fn new() -> Scanner {
        Scanner::default()
    }

    /// Whether to cut DA1 answers out as [`Segment::Da1`], for an SDK that
    /// reads DA1 itself (SDK.md §4.1).
    #[must_use]
    pub fn da1(mut self, on: bool) -> Scanner {
        self.da1 = on;
        self
    }

    /// The sequences it dropped as malformed: interrupted by an ESC that
    /// does not begin ST, longer than [`SCAN_MAX`], or unfinished at
    /// [`flush`](Scanner::flush).
    pub fn invalid(&self) -> usize {
        self.invalid
    }

    /// What it holds for the next feed.
    pub fn holding(&self) -> &[u8] {
        &self.held
    }

    /// Whether what it holds is a hotty sequence in progress (or one too
    /// long, being dropped), which the next feed goes on with, rather than
    /// the start of a segment, which a program reading keys may flush after
    /// a moment as typing.
    pub fn in_sequence(&self) -> bool {
        self.mode != Mode::Ground
    }

    /// Takes the next bytes of the stream, and returns the segments they
    /// complete, in order.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Segment> {
        let mut i = 0;
        while i < data.len() {
            if self.mode == Mode::Ground && self.held.is_empty() {
                let j = data[i..]
                    .iter()
                    .position(|&b| b == ESC)
                    .map_or(data.len(), |p| i + p);
                if j > i {
                    self.pass.extend_from_slice(&data[i..j]);
                    i = j;
                    continue;
                }
            }
            self.byte(data[i]);
            i += 1;
        }
        self.take()
    }

    /// Ends the stream: what is held goes out as [`Segment::Pass`], except
    /// a hotty sequence in progress, which is dropped as malformed. A
    /// program reading keys flushes a lone ESC held at the end of a read, as
    /// the Escape key, once nothing has followed it for a moment.
    pub fn flush(&mut self) -> Vec<Segment> {
        match self.mode {
            Mode::Osc => self.invalid += 1,
            Mode::Ground => {
                let held = std::mem::take(&mut self.held);
                self.pass.extend_from_slice(&held);
            }
            Mode::Discard => {}
        }
        self.mode = Mode::Ground;
        self.held.clear();
        self.esc = false;
        self.take()
    }

    fn take(&mut self) -> Vec<Segment> {
        self.end_pass();
        std::mem::take(&mut self.out)
    }

    fn end_pass(&mut self) {
        if !self.pass.is_empty() {
            self.out.push(Segment::Pass(std::mem::take(&mut self.pass)));
        }
    }

    fn emit(&mut self, seg: Segment) {
        self.end_pass();
        self.out.push(seg);
    }

    fn byte(&mut self, c: u8) {
        match self.mode {
            Mode::Discard => {
                if self.esc {
                    self.esc = false;
                    self.mode = Mode::Ground;
                    if c != b'\\' {
                        self.held.push(ESC);
                        self.ground(c);
                    }
                } else if c == BEL {
                    self.mode = Mode::Ground;
                } else if c == ESC {
                    self.esc = true;
                }
            }
            Mode::Osc => {
                if self.esc {
                    self.esc = false;
                    if c == b'\\' {
                        self.held.push(c);
                        let seq = std::mem::take(&mut self.held);
                        self.emit(Segment::Osc(seq));
                        self.mode = Mode::Ground;
                        return;
                    }
                    self.invalid += 1;
                    self.mode = Mode::Ground;
                    self.held.clear();
                    self.held.push(ESC);
                    self.ground(c);
                    return;
                }
                if c == BEL {
                    self.held.push(c);
                    let seq = std::mem::take(&mut self.held);
                    self.emit(Segment::Osc(seq));
                    self.mode = Mode::Ground;
                    return;
                }
                if self.held.len() >= SCAN_MAX {
                    self.invalid += 1;
                    self.mode = Mode::Discard;
                    self.held.clear();
                    self.esc = c == ESC;
                    return;
                }
                self.esc = c == ESC;
                self.held.push(c);
            }
            Mode::Ground => self.ground(c),
        }
    }

    fn ground(&mut self, c: u8) {
        if !self.held.is_empty() {
            self.held.push(c);
            if PREFIX.starts_with(&self.held) {
                if self.held.len() == PREFIX.len() {
                    self.mode = Mode::Osc;
                }
                return;
            }
            if self.da1 && self.da1_step(c) {
                return;
            }
            self.held.pop();
            let held = std::mem::take(&mut self.held);
            self.pass.extend_from_slice(&held);
        }
        if c == ESC {
            self.held.push(c);
        } else {
            self.pass.push(c);
        }
    }

    // Whether the held bytes, c the last of them, are still a DA1 answer in
    // progress, or a whole one, which goes out.
    fn da1_step(&mut self, c: u8) -> bool {
        if DA1_PREFIX.starts_with(&self.held) {
            return true;
        }
        if self.held.len() > DA1_PREFIX.len() && self.held.starts_with(DA1_PREFIX) {
            if c == b'c' {
                let seq = std::mem::take(&mut self.held);
                self.emit(Segment::Da1(seq));
                return true;
            }
            return c.is_ascii_digit() || c == b';';
        }
        false
    }
}
