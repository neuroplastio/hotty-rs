//! The terminal's input as crossterm's events.
//!
//! A hotty host sends its replies and events on the program's input, among
//! the keys, as OSC sequences. crossterm's reader would take those for
//! keys, so hotty-ratatui reads the terminal itself: hotty's sequences go
//! to the hotty SDK, and the rest goes to a [`Parser`], which turns it into
//! the [`Event`]s crossterm would have made of it. A program keeps its
//! crossterm handlers as they are.
//!
//! The parser reads what a terminal in raw mode sends:
//! - keys as characters and C0 controls, `ESC` before a key for Alt;
//! - cursor, editing and function keys, as `CSI` and `SS3` sequences with
//!   xterm's modifiers, and as the kitty keyboard protocol's `CSI … u`,
//!   with its event types (press, repeat, release), alternate keys and
//!   lock states; xterm's `modifyOtherKeys` (`CSI 27 ; mod ; code ~`);
//! - the mouse, in the SGR (1006), normal and rxvt (1015) encodings;
//! - bracketed paste, as one [`Event::Paste`];
//! - focus, as [`Event::FocusGained`] and [`Event::FocusLost`];
//! - in-band resize (mode 2048), as [`Event::Resize`].
//!
//! The terminal's answers to questions (DA1, DA2, the cursor's position,
//! the kitty keyboard flags, DECRPM, window reports) and its strings (OSC,
//! DCS, APC, PM, SOS) are not keys, and it swallows them.

use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MediaKeyCode,
    ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// The end of a bracketed paste.
const PASTE_END: &[u8] = b"\x1b[201~";

/// The longest `CSI` sequence it holds while waiting for its final byte;
/// one longer is dropped. Terminals send far shorter ones.
const CSI_MAX: usize = 4096;

/// Turns the terminal's input into crossterm's [`Event`]s, however the
/// reads split it.
///
/// It holds only what may still become a sequence: the start of one, a
/// character cut between two reads, a paste or a terminal string in
/// progress. Everything else comes out of the [`feed`](Parser::feed) that
/// brought it.
///
/// A lone `ESC` is the Escape key or the start of a sequence, which only
/// what follows tells. A reader feeds each read, and when the parser is
/// [`holding`](Parser::holding) but not [`in_sequence`](Parser::in_sequence),
/// flushes it after a moment with no more input: `ESC` alone is then
/// Escape, and `ESC [` is Alt+`[`. At the end of the input it flushes
/// whatever it holds.
///
/// ```
/// use hotty_ratatui::input::Parser;
/// use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
///
/// let mut p = Parser::new();
/// assert_eq!(
///     p.feed(b"a\x1b[1;5A"),
///     [
///         Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
///         Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
///     ]
/// );
/// assert!(p.feed(b"\x1b").is_empty());
/// assert!(p.holding() && !p.in_sequence());
/// assert_eq!(p.flush(), [Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))]);
/// ```
#[derive(Debug, Default)]
pub struct Parser {
    held: Vec<u8>,
    mode: Mode,
}

#[derive(Debug, Default)]
enum Mode {
    #[default]
    Keys,
    /// Inside a bracketed paste: what it has so far.
    Paste(Vec<u8>),
    /// Inside a terminal string (OSC, DCS, APC, PM, SOS), skipped to its end.
    Str,
}

/// What the parser made of the bytes at the head of what it holds.
enum Step {
    /// An event, from this many bytes.
    Event(Event, usize),
    /// Nothing for the program: this many bytes are dropped.
    Skip(usize),
    /// The bytes may still become a sequence: wait for more.
    More,
    /// A bracketed paste begins after this many bytes.
    Paste(usize),
    /// A terminal string begins after this many bytes.
    Str(usize),
}

impl Parser {
    /// A parser that holds nothing.
    pub fn new() -> Parser {
        Parser::default()
    }

    /// Takes the next bytes of the input, however the reads split it, and
    /// returns the events complete so far, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        self.held.extend_from_slice(bytes);
        self.run(&mut out, false);
        out
    }

    /// Whether it holds anything: the start of a sequence, a character cut
    /// short, or a paste or terminal string in progress.
    pub fn holding(&self) -> bool {
        !self.held.is_empty() || !matches!(self.mode, Mode::Keys)
    }

    /// Whether what it holds is a bracketed paste or a terminal string in
    /// progress, which only more input ends: a reader that flushes after a
    /// moment with no input leaves it alone, and flushes it only at the end
    /// of the input.
    pub fn in_sequence(&self) -> bool {
        !matches!(self.mode, Mode::Keys)
    }

    /// Ends what it holds, as if no more input will come: a lone `ESC` is
    /// Escape, the start of a sequence goes out as the keys it is (`ESC [`
    /// is Alt+`[`), a paste in progress goes out with what it has, and a
    /// terminal string in progress, or a character cut short, is dropped.
    pub fn flush(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        match std::mem::take(&mut self.mode) {
            Mode::Keys => {}
            Mode::Paste(mut p) => {
                p.append(&mut self.held);
                out.push(Event::Paste(String::from_utf8_lossy(&p).into_owned()));
            }
            Mode::Str => self.held.clear(),
        }
        self.run(&mut out, true);
        out
    }

    fn run(&mut self, out: &mut Vec<Event>, end: bool) {
        let mut buf = std::mem::take(&mut self.held);
        let mut i = 0;
        loop {
            match &mut self.mode {
                Mode::Paste(p) => {
                    let from = p.len().saturating_sub(PASTE_END.len() - 1);
                    p.extend_from_slice(&buf[i..]);
                    i = buf.len();
                    let Some(at) = find(&p[from..], PASTE_END).map(|at| from + at) else {
                        break;
                    };
                    let rest = p.split_off(at + PASTE_END.len());
                    p.truncate(at);
                    let text = String::from_utf8_lossy(p).into_owned();
                    out.push(Event::Paste(text));
                    self.mode = Mode::Keys;
                    buf = rest;
                    i = 0;
                }
                Mode::Str => match skip_string(&buf[i..]) {
                    StrEnd::Ended(n) => {
                        i += n;
                        self.mode = Mode::Keys;
                    }
                    StrEnd::Aborted(n) => {
                        i += n;
                        self.mode = Mode::Keys;
                    }
                    StrEnd::More(n) => {
                        i += n;
                        break;
                    }
                },
                Mode::Keys => {
                    if i >= buf.len() {
                        break;
                    }
                    match parse(&buf[i..]) {
                        Step::Event(ev, n) => {
                            out.push(ev);
                            i += n;
                        }
                        Step::Skip(n) => i += n,
                        Step::Paste(n) => {
                            i += n;
                            self.mode = Mode::Paste(Vec::new());
                        }
                        Step::Str(n) => {
                            i += n;
                            self.mode = Mode::Str;
                        }
                        Step::More if !end => break,
                        Step::More => {
                            let (ev, n) = cut_short(&buf[i..]);
                            out.extend(ev);
                            i += n;
                        }
                    }
                }
            }
        }
        buf.drain(..i);
        self.held = buf;
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

enum StrEnd {
    /// The string ended (BEL or ST) after this many bytes.
    Ended(usize),
    /// The string was cut off (CAN, SUB, or an ESC that begins no ST): this
    /// many bytes are its; what follows is input of its own.
    Aborted(usize),
    /// The string goes on: this many bytes are its, and the rest, an ESC at
    /// the end, waits for what follows.
    More(usize),
}

fn skip_string(b: &[u8]) -> StrEnd {
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            BEL => return StrEnd::Ended(i + 1),
            CAN | SUB => return StrEnd::Aborted(i + 1),
            ESC => match b.get(i + 1) {
                Some(b'\\') => return StrEnd::Ended(i + 2),
                Some(_) => return StrEnd::Aborted(i),
                None => return StrEnd::More(i),
            },
            _ => i += 1,
        }
    }
    StrEnd::More(i)
}

/// The keys a sequence cut short is, once no more input will come: at its
/// head, a lone `ESC` is Escape, `ESC` and a byte is Alt with that byte's
/// key, and a character cut short is dropped.
fn cut_short(b: &[u8]) -> (Option<Event>, usize) {
    if b[0] != ESC {
        return (None, 1);
    }
    match b.get(1) {
        Some(&c) if c < 0x80 => (Some(Event::Key(alt(ascii_key(c)))), 2),
        _ => (Some(key(KeyCode::Esc, KeyModifiers::NONE)), 1),
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn alt(mut k: KeyEvent) -> KeyEvent {
    k.modifiers |= KeyModifiers::ALT;
    k
}

/// A character typed: Shift with a capital, as crossterm has it.
fn char_key(c: char) -> KeyEvent {
    let m = if c.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(c), m)
}

/// The key one byte below 0x80 is, but `ESC`, in raw mode.
fn ascii_key(c: u8) -> KeyEvent {
    match c {
        b'\r' => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        b'\t' => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        0x7f => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        ESC => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        0 => KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL),
        0x01..=0x1a => KeyEvent::new(KeyCode::Char((b'a' + c - 1) as char), KeyModifiers::CONTROL),
        0x1c..=0x1f => KeyEvent::new(
            KeyCode::Char((b'4' + c - 0x1c) as char),
            KeyModifiers::CONTROL,
        ),
        _ => char_key(c as char),
    }
}

/// How many bytes a UTF-8 character that starts with `lead` takes; `None`
/// for a byte no character starts with.
fn utf8_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

enum Utf8 {
    Is(char, usize),
    More,
    Invalid,
}

fn utf8_char(b: &[u8]) -> Utf8 {
    let Some(n) = utf8_len(b[0]) else {
        return Utf8::Invalid;
    };
    if b.len() < n {
        return if b[1..].iter().all(|&c| c & 0xc0 == 0x80) {
            Utf8::More
        } else {
            Utf8::Invalid
        };
    }
    match std::str::from_utf8(&b[..n]) {
        Ok(s) => Utf8::Is(s.chars().next().unwrap_or_default(), n),
        Err(_) => Utf8::Invalid,
    }
}

fn parse(b: &[u8]) -> Step {
    match b[0] {
        ESC => parse_esc(b),
        c if c < 0x80 => Step::Event(Event::Key(ascii_key(c)), 1),
        _ => match utf8_char(b) {
            Utf8::Is(c, n) => Step::Event(Event::Key(char_key(c)), n),
            Utf8::More => Step::More,
            Utf8::Invalid => Step::Skip(1),
        },
    }
}

fn parse_esc(b: &[u8]) -> Step {
    let Some(&c) = b.get(1) else {
        return Step::More;
    };
    match c {
        b'[' => parse_csi(b),
        b'O' => parse_ss3(b),
        ESC => Step::Event(key(KeyCode::Esc, KeyModifiers::NONE), 2),
        // OSC, DCS, APC, PM and SOS begin strings; alone, they may be Alt
        // with ], P, _, ^ or X, which a flush tells.
        b']' | b'P' | b'_' | b'^' | b'X' => {
            if b.len() > 2 {
                Step::Str(2)
            } else {
                Step::More
            }
        }
        c if c < 0x80 => Step::Event(Event::Key(alt(ascii_key(c))), 2),
        _ => match utf8_char(&b[1..]) {
            Utf8::Is(ch, n) => Step::Event(Event::Key(alt(char_key(ch))), 1 + n),
            Utf8::More => Step::More,
            Utf8::Invalid => Step::Event(key(KeyCode::Esc, KeyModifiers::NONE), 1),
        },
    }
}

/// `SS3`: `ESC O` and a byte, the cursor and keypad keys of application
/// mode, and F1–F4.
fn parse_ss3(b: &[u8]) -> Step {
    let Some(&c) = b.get(2) else {
        return Step::More;
    };
    let code = match c {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'E' => KeyCode::KeypadBegin,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => {
            let pad = match c {
                b'M' => KeyCode::Enter,
                b'X' => KeyCode::Char('='),
                b'j' => KeyCode::Char('*'),
                b'k' => KeyCode::Char('+'),
                b'l' => KeyCode::Char(','),
                b'm' => KeyCode::Char('-'),
                b'n' => KeyCode::Char('.'),
                b'o' => KeyCode::Char('/'),
                b'p'..=b'y' => KeyCode::Char((b'0' + c - b'p') as char),
                // Alt+Shift+O, and the next byte a key of its own.
                _ => return Step::Event(Event::Key(alt(char_key('O'))), 2),
            };
            return Step::Event(
                Event::Key(KeyEvent::new_with_kind_and_state(
                    pad,
                    KeyModifiers::NONE,
                    KeyEventKind::Press,
                    KeyEventState::KEYPAD,
                )),
                3,
            );
        }
    };
    Step::Event(key(code, KeyModifiers::NONE), 3)
}

fn parse_csi(b: &[u8]) -> Step {
    let Some(&first) = b.get(2) else {
        return Step::More;
    };
    match first {
        // The normal mouse encoding: three bytes of their own, any values.
        b'M' => {
            if b.len() < 6 {
                return Step::More;
            }
            return match normal_mouse(b[3], b[4], b[5]) {
                Some(ev) => Step::Event(ev, 6),
                None => Step::Skip(6),
            };
        }
        // The Linux console's F1–F5.
        b'[' => {
            return match b.get(3) {
                None => Step::More,
                Some(&c @ b'A'..=b'E') => {
                    Step::Event(key(KeyCode::F(1 + c - b'A'), KeyModifiers::NONE), 4)
                }
                Some(_) => Step::Skip(3),
            };
        }
        _ => {}
    }
    let mut j = 2;
    while j < b.len() {
        match b[j] {
            0x20..=0x3f => j += 1,
            0x40..=0x7e => break,
            // Not a CSI byte: the sequence is broken, and this byte is input of its own.
            _ => return Step::Skip(j),
        }
    }
    if j >= b.len() {
        return if j > CSI_MAX {
            Step::Skip(j)
        } else {
            Step::More
        };
    }
    let body = &b[2..j];
    let fin = b[j];
    let n = j + 1;
    let (private, rest) = match body.first() {
        Some(&p @ b'<'..=b'?') => (Some(p), &body[1..]),
        _ => (None, body),
    };
    let split = rest
        .iter()
        .position(|c| (0x20..=0x2f).contains(c))
        .unwrap_or(rest.len());
    let (params, inter) = rest.split_at(split);
    if private.is_none() && inter.is_empty() && fin == b'~' && params == b"200" {
        return Step::Paste(n);
    }
    match csi(private, &Params::read(params), inter, fin) {
        Some(ev) => Step::Event(ev, n),
        None => Step::Skip(n),
    }
}

/// A CSI sequence's parameters: `;` between them, `:` between a
/// parameter's parts; an empty part is absent.
struct Params(Vec<Vec<Option<u32>>>);

impl Params {
    fn read(p: &[u8]) -> Params {
        if p.is_empty() {
            return Params(Vec::new());
        }
        Params(
            p.split(|&c| c == b';')
                .map(|f| {
                    f.split(|&c| c == b':')
                        .map(|s| std::str::from_utf8(s).ok()?.parse().ok())
                        .collect()
                })
                .collect(),
        )
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn get(&self, i: usize, part: usize) -> Option<u32> {
        *self.0.get(i)?.get(part)?
    }
}

/// What a CSI sequence is: an event, or `None` for one that is not the
/// program's (an answer, or one it does not know).
fn csi(private: Option<u8>, p: &Params, inter: &[u8], fin: u8) -> Option<Event> {
    if private == Some(b'<') && inter.is_empty() && (fin == b'M' || fin == b'm') {
        return sgr_mouse(p, fin == b'm');
    }
    // Answers: DA1 and DA2, the kitty keyboard flags, DECRPM, and others.
    if private.is_some() || !inter.is_empty() {
        return None;
    }
    match fin {
        b'A' | b'B' | b'C' | b'D' | b'H' | b'F' | b'E' | b'P' | b'Q' | b'S' => {
            let code = match fin {
                b'A' => KeyCode::Up,
                b'B' => KeyCode::Down,
                b'C' => KeyCode::Right,
                b'D' => KeyCode::Left,
                b'H' => KeyCode::Home,
                b'F' => KeyCode::End,
                b'E' => KeyCode::KeypadBegin,
                b'P' => KeyCode::F(1),
                b'Q' => KeyCode::F(2),
                _ => KeyCode::F(4),
            };
            // `CSI 1 ; mods X`, and an older `CSI mods X`.
            let m = if p.len() >= 2 { 1 } else { 0 };
            let (mods, kind, state) = modifiers(p.get(m, 0), p.get(m, 1));
            Some(Event::Key(KeyEvent::new_with_kind_and_state(
                code, mods, kind, state,
            )))
        }
        b'Z' => {
            let (mods, kind, state) = modifiers(p.get(1, 0), p.get(1, 1));
            Some(Event::Key(KeyEvent::new_with_kind_and_state(
                KeyCode::BackTab,
                mods | KeyModifiers::SHIFT,
                kind,
                state,
            )))
        }
        b'I' if p.is_empty() => Some(Event::FocusGained),
        b'O' if p.is_empty() => Some(Event::FocusLost),
        b'~' => tilde_key(p),
        b'u' => kitty_key(p),
        b'M' if p.len() == 3 => rxvt_mouse(p),
        b't' if p.get(0, 0) == Some(48) => {
            let rows = clamp(p.get(1, 0)?);
            let cols = clamp(p.get(2, 0)?);
            Some(Event::Resize(cols, rows))
        }
        // R is the cursor's position, n a status report, t a window's.
        _ => None,
    }
}

fn clamp(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

/// The modifiers, kind and lock state of a key, from its `mods` parameter
/// (1 more than the bits: Shift 1, Alt 2, Ctrl 4, Super 8, Hyper 16, Meta
/// 32, Caps Lock 64, Num Lock 128) and its event type (1 press, 2 repeat,
/// 3 release).
fn modifiers(mods: Option<u32>, kind: Option<u32>) -> (KeyModifiers, KeyEventKind, KeyEventState) {
    let bits = mods.unwrap_or(1).saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    for (bit, flag) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if bits & bit != 0 {
            m |= flag;
        }
    }
    let mut state = KeyEventState::NONE;
    if bits & 64 != 0 {
        state |= KeyEventState::CAPS_LOCK;
    }
    if bits & 128 != 0 {
        state |= KeyEventState::NUM_LOCK;
    }
    let kind = match kind {
        Some(2) => KeyEventKind::Repeat,
        Some(3) => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };
    (m, kind, state)
}

/// `CSI n ; mods ~`: the editing keys and F5 on, and xterm's
/// `modifyOtherKeys`, `CSI 27 ; mods ; code ~`.
fn tilde_key(p: &Params) -> Option<Event> {
    let n = p.get(0, 0)?;
    if n == 27 {
        let (mods, kind, state) = modifiers(p.get(1, 0), p.get(1, 1));
        let code = codepoint_key(p.get(2, 0)?, mods)?;
        return Some(Event::Key(KeyEvent::new_with_kind_and_state(
            code, mods, kind, state,
        )));
    }
    let code = match n {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        11..=15 => KeyCode::F((n - 10) as u8),
        17..=21 => KeyCode::F((n - 11) as u8),
        23..=26 => KeyCode::F((n - 12) as u8),
        28 | 29 => KeyCode::F((n - 13) as u8),
        31..=34 => KeyCode::F((n - 14) as u8),
        _ => return None,
    };
    let (mods, kind, state) = modifiers(p.get(1, 0), p.get(1, 1));
    Some(Event::Key(KeyEvent::new_with_kind_and_state(
        code, mods, kind, state,
    )))
}

/// The key a codepoint names, in the kitty protocol and `modifyOtherKeys`.
fn codepoint_key(cp: u32, mods: KeyModifiers) -> Option<KeyCode> {
    Some(match cp {
        27 => KeyCode::Esc,
        13 => KeyCode::Enter,
        9 if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        9 => KeyCode::Tab,
        127 => KeyCode::Backspace,
        _ => KeyCode::Char(char::from_u32(cp)?),
    })
}

/// The kitty protocol's keys of its private range (its "functional" keys):
/// the keypad's, with the keypad state, and the rest.
fn functional_key(cp: u32) -> Option<(KeyCode, KeyEventState)> {
    let pad = |c| Some((c, KeyEventState::KEYPAD));
    let plain = |c| Some((c, KeyEventState::NONE));
    match cp {
        57399..=57408 => pad(KeyCode::Char(char::from(b'0' + (cp - 57399) as u8))),
        57409 => pad(KeyCode::Char('.')),
        57410 => pad(KeyCode::Char('/')),
        57411 => pad(KeyCode::Char('*')),
        57412 => pad(KeyCode::Char('-')),
        57413 => pad(KeyCode::Char('+')),
        57414 => pad(KeyCode::Enter),
        57415 => pad(KeyCode::Char('=')),
        57416 => pad(KeyCode::Char(',')),
        57417 => pad(KeyCode::Left),
        57418 => pad(KeyCode::Right),
        57419 => pad(KeyCode::Up),
        57420 => pad(KeyCode::Down),
        57421 => pad(KeyCode::PageUp),
        57422 => pad(KeyCode::PageDown),
        57423 => pad(KeyCode::Home),
        57424 => pad(KeyCode::End),
        57425 => pad(KeyCode::Insert),
        57426 => pad(KeyCode::Delete),
        57427 => pad(KeyCode::KeypadBegin),
        57358 => plain(KeyCode::CapsLock),
        57359 => plain(KeyCode::ScrollLock),
        57360 => plain(KeyCode::NumLock),
        57361 => plain(KeyCode::PrintScreen),
        57362 => plain(KeyCode::Pause),
        57363 => plain(KeyCode::Menu),
        57376..=57398 => plain(KeyCode::F((cp - 57376 + 13) as u8)),
        57428..=57440 => plain(KeyCode::Media(
            [
                MediaKeyCode::Play,
                MediaKeyCode::Pause,
                MediaKeyCode::PlayPause,
                MediaKeyCode::Reverse,
                MediaKeyCode::Stop,
                MediaKeyCode::FastForward,
                MediaKeyCode::Rewind,
                MediaKeyCode::TrackNext,
                MediaKeyCode::TrackPrevious,
                MediaKeyCode::Record,
                MediaKeyCode::LowerVolume,
                MediaKeyCode::RaiseVolume,
                MediaKeyCode::MuteVolume,
            ][(cp - 57428) as usize],
        )),
        57441..=57454 => plain(KeyCode::Modifier(
            [
                ModifierKeyCode::LeftShift,
                ModifierKeyCode::LeftControl,
                ModifierKeyCode::LeftAlt,
                ModifierKeyCode::LeftSuper,
                ModifierKeyCode::LeftHyper,
                ModifierKeyCode::LeftMeta,
                ModifierKeyCode::RightShift,
                ModifierKeyCode::RightControl,
                ModifierKeyCode::RightAlt,
                ModifierKeyCode::RightSuper,
                ModifierKeyCode::RightHyper,
                ModifierKeyCode::RightMeta,
                ModifierKeyCode::IsoLevel3Shift,
                ModifierKeyCode::IsoLevel5Shift,
            ][(cp - 57441) as usize],
        )),
        _ => None,
    }
}

/// The kitty keyboard protocol's `CSI code[:shifted[:base]] ; mods[:event]
/// ; text u`. A modifier key pressed carries its own modifier, and with
/// Shift a key that reports its shifted form is that form, without Shift,
/// as crossterm has them.
fn kitty_key(p: &Params) -> Option<Event> {
    let cp = p.get(0, 0)?;
    let (mut mods, kind, lock) = modifiers(p.get(1, 0), p.get(1, 1));
    let (mut code, pad) = match functional_key(cp) {
        Some(k) => k,
        None => (codepoint_key(cp, mods)?, KeyEventState::NONE),
    };
    if let KeyCode::Modifier(m) = code {
        mods |= match m {
            ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift => KeyModifiers::SHIFT,
            ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl => KeyModifiers::CONTROL,
            ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt => KeyModifiers::ALT,
            ModifierKeyCode::LeftSuper | ModifierKeyCode::RightSuper => KeyModifiers::SUPER,
            ModifierKeyCode::LeftHyper | ModifierKeyCode::RightHyper => KeyModifiers::HYPER,
            ModifierKeyCode::LeftMeta | ModifierKeyCode::RightMeta => KeyModifiers::META,
            _ => KeyModifiers::NONE,
        };
    }
    if mods.contains(KeyModifiers::SHIFT)
        && let Some(shifted) = p.get(0, 1).and_then(char::from_u32)
    {
        code = KeyCode::Char(shifted);
        mods.remove(KeyModifiers::SHIFT);
    }
    Some(Event::Key(KeyEvent::new_with_kind_and_state(
        code,
        mods,
        kind,
        pad | lock,
    )))
}

/// A mouse report's button byte, less its offset: the button, the
/// modifiers held, and whether the pointer moved.
fn mouse_kind(cb: u32) -> Option<(MouseEventKind, KeyModifiers)> {
    let button = (cb & 0b11) | ((cb & 0b1100_0000) >> 4);
    let moved = cb & 0b10_0000 != 0;
    let kind = match (button, moved) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        // The buttons crossterm has no name for: back, forward, and on.
        _ => return None,
    };
    let mut m = KeyModifiers::NONE;
    if cb & 4 != 0 {
        m |= KeyModifiers::SHIFT;
    }
    if cb & 8 != 0 {
        m |= KeyModifiers::ALT;
    }
    if cb & 16 != 0 {
        m |= KeyModifiers::CONTROL;
    }
    Some((kind, m))
}

fn mouse(cb: u32, x: u32, y: u32) -> Option<Event> {
    let (kind, modifiers) = mouse_kind(cb)?;
    Some(Event::Mouse(MouseEvent {
        kind,
        column: clamp(x.saturating_sub(1)),
        row: clamp(y.saturating_sub(1)),
        modifiers,
    }))
}

/// SGR (1006): `CSI < b ; x ; y M` for a press or a motion, `m` for a release.
fn sgr_mouse(p: &Params, release: bool) -> Option<Event> {
    if p.len() != 3 {
        return None;
    }
    let mut ev = mouse(p.get(0, 0)?, p.get(1, 0)?, p.get(2, 0)?)?;
    if release
        && let Event::Mouse(m) = &mut ev
        && let MouseEventKind::Down(b) = m.kind
    {
        m.kind = MouseEventKind::Up(b);
    }
    Some(ev)
}

/// The normal encoding: `CSI M` and three bytes, each 32 more than its value.
fn normal_mouse(cb: u8, x: u8, y: u8) -> Option<Event> {
    let cb = u32::from(cb.checked_sub(32)?);
    mouse(
        cb,
        u32::from(x.saturating_sub(32)),
        u32::from(y.saturating_sub(32)),
    )
}

/// rxvt's (1015): `CSI b ; x ; y M`, the button 32 more than its value.
fn rxvt_mouse(p: &Params) -> Option<Event> {
    mouse(p.get(0, 0)?.checked_sub(32)?, p.get(1, 0)?, p.get(2, 0)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    fn k(code: KeyCode, m: KeyModifiers) -> Event {
        key(code, m)
    }

    fn c(ch: char) -> Event {
        k(KeyCode::Char(ch), NONE)
    }

    fn kk(code: KeyCode, m: KeyModifiers, kind: KeyEventKind, state: KeyEventState) -> Event {
        Event::Key(KeyEvent::new_with_kind_and_state(code, m, kind, state))
    }

    fn ms(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers,
        })
    }

    fn paste(s: &str) -> Event {
        Event::Paste(s.to_string())
    }

    const PRESS: KeyEventKind = KeyEventKind::Press;
    const RELEASE: KeyEventKind = KeyEventKind::Release;
    const REPEAT: KeyEventKind = KeyEventKind::Repeat;
    const NOSTATE: KeyEventState = KeyEventState::NONE;
    const KEYPAD: KeyEventState = KeyEventState::KEYPAD;

    /// Every case: the input, and the events it is once flushed.
    fn cases() -> Vec<(&'static [u8], Vec<Event>)> {
        use KeyCode::*;
        use MouseButton::{Left as LeftB, Middle as MiddleB, Right as RightB};
        use MouseEventKind as M;
        vec![
            // Characters.
            (b"a", vec![c('a')]),
            (b"A", vec![k(Char('A'), SHIFT)]),
            (b" ~", vec![c(' '), c('~')]),
            ("é".as_bytes(), vec![c('é')]),
            ("É".as_bytes(), vec![k(Char('É'), SHIFT)]),
            ("日本".as_bytes(), vec![c('日'), c('本')]),
            ("🦀x".as_bytes(), vec![c('🦀'), c('x')]),
            // C0 controls, in raw mode.
            (b"\r", vec![k(Enter, NONE)]),
            (b"\t", vec![k(Tab, NONE)]),
            (b"\x7f", vec![k(Backspace, NONE)]),
            (b"\x08", vec![k(Char('h'), CTRL)]),
            (b"\n", vec![k(Char('j'), CTRL)]),
            (b"\x00", vec![k(Char(' '), CTRL)]),
            (b"\x01\x1a", vec![k(Char('a'), CTRL), k(Char('z'), CTRL)]),
            (b"\x1c\x1f", vec![k(Char('4'), CTRL), k(Char('7'), CTRL)]),
            // Escape and Alt.
            (b"\x1b", vec![k(Esc, NONE)]),
            (b"\x1b\x1b", vec![k(Esc, NONE)]),
            (b"\x1ba", vec![k(Char('a'), ALT)]),
            (b"\x1bA", vec![k(Char('A'), ALT | SHIFT)]),
            (b"\x1b\r", vec![k(Enter, ALT)]),
            (b"\x1b\x7f", vec![k(Backspace, ALT)]),
            (b"\x1b\x01", vec![k(Char('a'), ALT | CTRL)]),
            ("\x1bé".as_bytes(), vec![k(Char('é'), ALT)]),
            (b"\x1b[", vec![k(Char('['), ALT)]),
            (b"\x1b[1;", vec![k(Char('['), ALT), c('1'), c(';')]),
            (b"\x1bO", vec![k(Char('O'), ALT | SHIFT)]),
            (b"\x1bOz", vec![k(Char('O'), ALT | SHIFT), c('z')]),
            (b"\x1b]", vec![k(Char(']'), ALT)]),
            (b"\x1bP", vec![k(Char('P'), ALT | SHIFT)]),
            // Cursor and function keys.
            (
                b"\x1b[A\x1b[B\x1b[C\x1b[D",
                vec![k(Up, NONE), k(Down, NONE), k(Right, NONE), k(Left, NONE)],
            ),
            (b"\x1b[H\x1b[F", vec![k(Home, NONE), k(End, NONE)]),
            (b"\x1bOA\x1bOH", vec![k(Up, NONE), k(Home, NONE)]),
            (
                b"\x1bOP\x1bOQ\x1bOR\x1bOS",
                vec![k(F(1), NONE), k(F(2), NONE), k(F(3), NONE), k(F(4), NONE)],
            ),
            (
                b"\x1b[P\x1b[Q\x1b[S",
                vec![k(F(1), NONE), k(F(2), NONE), k(F(4), NONE)],
            ),
            (b"\x1b[1;5A", vec![k(Up, CTRL)]),
            (b"\x1b[1;2D", vec![k(Left, SHIFT)]),
            (b"\x1b[1;3H", vec![k(Home, ALT)]),
            (b"\x1b[1;8F", vec![k(End, SHIFT | ALT | CTRL)]),
            (b"\x1b[5A", vec![k(Up, CTRL)]),
            (b"\x1b[1;2P", vec![k(F(1), SHIFT)]),
            (b"\x1b[E", vec![k(KeypadBegin, NONE)]),
            (b"\x1b[[A\x1b[[E", vec![k(F(1), NONE), k(F(5), NONE)]),
            (b"\x1b[Z", vec![k(BackTab, SHIFT)]),
            (b"\x1b[1;5Z", vec![k(BackTab, SHIFT | CTRL)]),
            (b"\x1bOM", vec![kk(Enter, NONE, PRESS, KEYPAD)]),
            (
                b"\x1bOp\x1bOy\x1bOk",
                vec![
                    kk(Char('0'), NONE, PRESS, KEYPAD),
                    kk(Char('9'), NONE, PRESS, KEYPAD),
                    kk(Char('+'), NONE, PRESS, KEYPAD),
                ],
            ),
            // Event types on legacy keys (kitty).
            (b"\x1b[1;5:3A", vec![kk(Up, CTRL, RELEASE, NOSTATE)]),
            (b"\x1b[1;1:2B", vec![kk(Down, NONE, REPEAT, NOSTATE)]),
            (b"\x1b[3;1:3~", vec![kk(Delete, NONE, RELEASE, NOSTATE)]),
            // ~ keys.
            (b"\x1b[2~\x1b[3~", vec![k(Insert, NONE), k(Delete, NONE)]),
            (b"\x1b[5~\x1b[6~", vec![k(PageUp, NONE), k(PageDown, NONE)]),
            (
                b"\x1b[1~\x1b[7~\x1b[4~\x1b[8~",
                vec![k(Home, NONE), k(Home, NONE), k(End, NONE), k(End, NONE)],
            ),
            (b"\x1b[3;5~", vec![k(Delete, CTRL)]),
            (
                b"\x1b[11~\x1b[15~\x1b[17~\x1b[24~",
                vec![k(F(1), NONE), k(F(5), NONE), k(F(6), NONE), k(F(12), NONE)],
            ),
            (
                b"\x1b[25~\x1b[26~\x1b[28~\x1b[29~",
                vec![
                    k(F(13), NONE),
                    k(F(14), NONE),
                    k(F(15), NONE),
                    k(F(16), NONE),
                ],
            ),
            (b"\x1b[31~\x1b[34~", vec![k(F(17), NONE), k(F(20), NONE)]),
            (b"\x1b[99~x", vec![c('x')]),
            // modifyOtherKeys.
            (b"\x1b[27;5;105~", vec![k(Char('i'), CTRL)]),
            (b"\x1b[27;2;13~", vec![k(Enter, SHIFT)]),
            (b"\x1b[27;6;9~", vec![k(BackTab, SHIFT | CTRL)]),
            // The kitty keyboard protocol.
            (b"\x1b[97u", vec![c('a')]),
            (b"\x1b[97;5u", vec![k(Char('a'), CTRL)]),
            (b"\x1b[97;2u", vec![k(Char('a'), SHIFT)]),
            (b"\x1b[97:65;2u", vec![c('A')]),
            (b"\x1b[97:65:97;6u", vec![k(Char('A'), CTRL)]),
            (b"\x1b[97;1:3u", vec![kk(Char('a'), NONE, RELEASE, NOSTATE)]),
            (b"\x1b[97;;97u", vec![c('a')]),
            (
                b"\x1b[13u\x1b[27u\x1b[9u\x1b[127u",
                vec![
                    k(Enter, NONE),
                    k(Esc, NONE),
                    k(Tab, NONE),
                    k(Backspace, NONE),
                ],
            ),
            (b"\x1b[9;2u", vec![k(BackTab, SHIFT)]),
            (b"\x1b[127;5u", vec![k(Backspace, CTRL)]),
            (
                b"\x1b[57399u\x1b[57414u",
                vec![
                    kk(Char('0'), NONE, PRESS, KEYPAD),
                    kk(Enter, NONE, PRESS, KEYPAD),
                ],
            ),
            (
                b"\x1b[57441;2u",
                vec![k(Modifier(ModifierKeyCode::LeftShift), SHIFT)],
            ),
            (
                b"\x1b[57448u",
                vec![k(Modifier(ModifierKeyCode::RightControl), CTRL)],
            ),
            (
                b"\x1b[57428u\x1b[57440u",
                vec![
                    k(Media(MediaKeyCode::Play), NONE),
                    k(Media(MediaKeyCode::MuteVolume), NONE),
                ],
            ),
            (
                b"\x1b[57376u\x1b[57398u",
                vec![k(F(13), NONE), k(F(35), NONE)],
            ),
            (
                b"\x1b[57358u\x1b[57363u",
                vec![k(CapsLock, NONE), k(Menu, NONE)],
            ),
            (
                b"\x1b[97;65u",
                vec![kk(Char('a'), NONE, PRESS, KeyEventState::CAPS_LOCK)],
            ),
            (
                b"\x1b[97;133u",
                vec![kk(Char('a'), CTRL, PRESS, KeyEventState::NUM_LOCK)],
            ),
            (
                b"\x1b[97;9u\x1b[97;17u\x1b[97;33u",
                vec![
                    k(Char('a'), KeyModifiers::SUPER),
                    k(Char('a'), KeyModifiers::HYPER),
                    k(Char('a'), KeyModifiers::META),
                ],
            ),
            (b"\x1b[55296ux", vec![c('x')]),
            // The mouse: SGR.
            (b"\x1b[<0;10;5M", vec![ms(M::Down(LeftB), 9, 4, NONE)]),
            (b"\x1b[<0;10;5m", vec![ms(M::Up(LeftB), 9, 4, NONE)]),
            (
                b"\x1b[<1;1;1M\x1b[<2;1;1M",
                vec![
                    ms(M::Down(MiddleB), 0, 0, NONE),
                    ms(M::Down(RightB), 0, 0, NONE),
                ],
            ),
            (b"\x1b[<2;3;3m", vec![ms(M::Up(RightB), 2, 2, NONE)]),
            (
                b"\x1b[<32;3;4M\x1b[<34;3;4M",
                vec![
                    ms(M::Drag(LeftB), 2, 3, NONE),
                    ms(M::Drag(RightB), 2, 3, NONE),
                ],
            ),
            (b"\x1b[<35;3;4M", vec![ms(M::Moved, 2, 3, NONE)]),
            (
                b"\x1b[<64;1;1M\x1b[<65;1;1M\x1b[<66;1;1M\x1b[<67;1;1M",
                vec![
                    ms(M::ScrollUp, 0, 0, NONE),
                    ms(M::ScrollDown, 0, 0, NONE),
                    ms(M::ScrollLeft, 0, 0, NONE),
                    ms(M::ScrollRight, 0, 0, NONE),
                ],
            ),
            (
                b"\x1b[<4;1;1M\x1b[<8;1;1M\x1b[<16;1;1M",
                vec![
                    ms(M::Down(LeftB), 0, 0, SHIFT),
                    ms(M::Down(LeftB), 0, 0, ALT),
                    ms(M::Down(LeftB), 0, 0, CTRL),
                ],
            ),
            (b"\x1b[<80;300;200M", vec![ms(M::ScrollUp, 299, 199, CTRL)]),
            (b"\x1b[<128;1;1Mx", vec![c('x')]),
            (b"\x1b[<0;1Mx", vec![c('x')]),
            // The mouse: normal and rxvt.
            (b"\x1b[M !!", vec![ms(M::Down(LeftB), 0, 0, NONE)]),
            (b"\x1b[M#!!", vec![ms(M::Up(LeftB), 0, 0, NONE)]),
            (b"\x1b[M@*+", vec![ms(M::Drag(LeftB), 9, 10, NONE)]),
            (
                b"\x1b[M`!!\x1b[Ma!!",
                vec![ms(M::ScrollUp, 0, 0, NONE), ms(M::ScrollDown, 0, 0, NONE)],
            ),
            (b"\x1b[M \xff\xff", vec![ms(M::Down(LeftB), 222, 222, NONE)]),
            (b"\x1b[M\x10!!x", vec![c('x')]),
            (b"\x1b[32;10;5M", vec![ms(M::Down(LeftB), 9, 4, NONE)]),
            (b"\x1b[35;1;2M", vec![ms(M::Up(LeftB), 0, 1, NONE)]),
            // Bracketed paste.
            (b"\x1b[200~hello\x1b[201~", vec![paste("hello")]),
            (b"\x1b[200~\x1b[201~", vec![paste("")]),
            (
                b"x\x1b[200~a\x1b[Ab\r\n\x1b[201~y",
                vec![c('x'), paste("a\x1b[Ab\r\n"), c('y')],
            ),
            (
                "\x1b[200~日本 🦀\x1b[201~".as_bytes(),
                vec![paste("日本 🦀")],
            ),
            (b"\x1b[200~a\x1b[201", vec![paste("a\x1b[201")]),
            (b"\x1b[201~x", vec![c('x')]),
            // Focus.
            (b"\x1b[I\x1b[O", vec![Event::FocusGained, Event::FocusLost]),
            // In-band resize (mode 2048).
            (b"\x1b[48;24;80;480;640t", vec![Event::Resize(80, 24)]),
            // Answers and strings: nothing.
            (b"a\x1b[?62;22cb", vec![c('a'), c('b')]),
            (b"\x1b[>1;10;0c", vec![]),
            (b"\x1b[12;40R\x1b[?12;40R", vec![]),
            (b"\x1b[?1u\x1b[?2026;2$y\x1b[2026;2$y", vec![]),
            (b"\x1b[0n\x1b[8;24;80t", vec![]),
            (b"\x1b]11;rgb:0000/0000/0000\x1b\\x", vec![c('x')]),
            (b"\x1b]11;rgb:ffff/ffff/ffff\x07x", vec![c('x')]),
            (b"\x1bP1$r0m\x1b\\\x1bP>|kitty\x1b\\x", vec![c('x')]),
            (b"\x1b_Gi=31;OK\x1b\\x", vec![c('x')]),
            (b"\x1b^pm\x1b\\\x1bXsos\x1b\\x", vec![c('x')]),
            (b"\x1b]0;unfinished", vec![]),
            (b"\x1b]11;ab\x1b[A", vec![k(Up, NONE)]),
            (b"\x1b]11;ab\x18x", vec![c('x')]),
            // What does not parse is dropped, and what follows it stays.
            (b"\x1b[1;\x01a", vec![k(Char('a'), CTRL), c('a')]),
            (b"\x1b[5xy", vec![c('y')]),
            (b"\x1b[[zq", vec![c('z'), c('q')]),
            (b"\xffa\x80b", vec![c('a'), c('b')]),
            (b"\xc3(", vec![c('(')]),
            (b"\xe6\x97", vec![]),
            (b"\x1b\xff", vec![k(Esc, NONE)]),
            (b"\x1b\xe6\x97", vec![k(Esc, NONE)]),
        ]
    }

    fn show(evs: &[Event]) -> String {
        format!("{evs:?}")
    }

    fn run(chunks: &[&[u8]]) -> Vec<Event> {
        let mut p = Parser::new();
        let mut out = Vec::new();
        for ch in chunks {
            out.extend(p.feed(ch));
        }
        out.extend(p.flush());
        assert!(!p.holding(), "holds {:?} after a flush", p.held);
        out
    }

    #[test]
    fn every_case_whole() {
        for (input, want) in cases() {
            assert_eq!(
                show(&run(&[input])),
                show(&want),
                "{:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn every_case_a_byte_at_a_time() {
        for (input, want) in cases() {
            let bytes: Vec<&[u8]> = input.chunks(1).collect();
            assert_eq!(
                show(&run(&bytes)),
                show(&want),
                "{:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn every_case_cut_at_every_offset() {
        for (input, want) in cases() {
            for at in 0..=input.len() {
                let (a, b) = input.split_at(at);
                assert_eq!(
                    show(&run(&[a, b])),
                    show(&want),
                    "{:?} cut at {at}",
                    String::from_utf8_lossy(input)
                );
            }
        }
    }

    #[test]
    fn all_the_cases_in_one_stream() {
        let mut all = Vec::new();
        let mut want = Vec::new();
        for (input, w) in cases() {
            // Cases that end held, or in a string, would swallow the next one.
            let mut p = Parser::new();
            p.feed(input);
            if p.holding() {
                continue;
            }
            all.extend_from_slice(input);
            want.extend(w);
        }
        assert_eq!(show(&run(&[&all])), show(&want));
        let bytes: Vec<&[u8]> = all.chunks(1).collect();
        assert_eq!(show(&run(&bytes)), show(&want));
        let sevens: Vec<&[u8]> = all.chunks(7).collect();
        assert_eq!(show(&run(&sevens)), show(&want));
    }

    #[test]
    fn a_key_never_waits_for_the_next_read() {
        let mut p = Parser::new();
        assert_eq!(
            show(&p.feed(b"ab\x1b[A")),
            show(&[c('a'), c('b'), k(KeyCode::Up, NONE)])
        );
        assert!(!p.holding());
        assert_eq!(show(&p.feed(b"x\x1b")), show(&[c('x')]));
        assert!(p.holding() && !p.in_sequence());
        assert_eq!(show(&p.feed(b"[B")), show(&[k(KeyCode::Down, NONE)]));
    }

    #[test]
    fn holding_and_in_sequence() {
        let mut p = Parser::new();
        for start in [
            &b"\x1b"[..],
            b"\x1b[",
            b"\x1b[1;5",
            b"\x1bO",
            b"\x1b]",
            b"\xe6",
            b"\x1b[M ",
            b"\x1b[<0;1",
        ] {
            p.feed(start);
            assert!(p.holding() && !p.in_sequence(), "{start:?}");
            p.flush();
            assert!(!p.holding());
        }
        p.feed(b"\x1b[200~ab");
        assert!(p.holding() && p.in_sequence());
        assert_eq!(show(&p.feed(b"c\x1b[201~")), show(&[paste("abc")]));
        assert!(!p.holding());
        p.feed(b"\x1b]11;");
        assert!(p.holding() && p.in_sequence());
        p.feed(b"rgb\x1b");
        assert!(p.in_sequence());
        assert!(p.feed(b"\\").is_empty());
        assert!(!p.holding());
    }

    #[test]
    fn a_long_paste_across_many_reads() {
        let text: String = (0..20_000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let mut input = b"\x1b[200~".to_vec();
        input.extend(text.as_bytes());
        input.extend(b"\x1b[201~z");
        let reads: Vec<&[u8]> = input.chunks(4095).collect();
        assert_eq!(show(&run(&reads)), show(&[paste(&text), c('z')]));
    }

    #[test]
    fn a_long_string_holds_nothing() {
        let mut p = Parser::new();
        p.feed(b"\x1b]52;c;");
        for _ in 0..1000 {
            p.feed(&[b'A'; 1024]);
        }
        assert!(p.held.is_empty());
        assert_eq!(show(&p.feed(b"\x07q")), show(&[c('q')]));
    }

    #[test]
    fn a_csi_that_never_ends_is_dropped() {
        let mut p = Parser::new();
        let mut input = b"\x1b[".to_vec();
        input.extend(std::iter::repeat_n(b'1', CSI_MAX));
        assert!(p.feed(&input).is_empty());
        assert!(p.held.len() <= CSI_MAX);
    }
}
