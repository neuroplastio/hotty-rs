//! Key names and keymaps (SDK.md §3.10; SPEC §10.2, §10.4).
//!
//! What a text field does with a key, as data, so that a program that draws
//! its fields in cells edits them as a host edits them on a surface, and
//! the keymap it gives a surface is the one it uses in cells:
//!
//! - [`parse_key`] reads a key's name as a document writes it, and
//!   [`decode_keys`] reads the keys in what the terminal sends the program;
//!   [`key_text`] is the text a key types.
//! - A [`Keymap`] binds keys to actions. [`resolve`] builds a field's: the
//!   default keymap for the [`Platform`], then each `data-keys` value, the
//!   root's first. [`Keymap::lookup`] says what the field does with a key.
//! - [`TERMINAL_KEYS`] is the SDK's keymap, the same in every SDK: a
//!   program puts it in the `data-keys` of an element that holds its
//!   fields, and resolves the same keymap for its cells.
//!
//! ```
//! use hotty::{INSERT, Platform, TERMINAL_KEYS, resolve};
//!
//! let keys = resolve(&Platform::default(), false, [TERMINAL_KEYS]);
//! assert_eq!(keys.lookup("ctrl+w"), Some("delete-word-backward"));
//! assert_eq!(keys.lookup("shift+a"), Some(INSERT));
//! assert_eq!(keys.lookup("tab"), None); // not the field's: it moves focus
//! ```
//!
//! The key and keymap code comes from hotty-blitz's `hotty-wire`, which
//! does the same for a host.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::messages::Caps;

/// The modifier keys, in the order a key's name writes them (myelin key
/// names §2; SPEC §10.4).
pub const MODIFIERS: [&str; 4] = ["super", "ctrl", "alt", "shift"];

/// The scroll actions (SPEC §10.2, scrolling keys), which a text field's
/// keymap leaves out.
pub const SCROLL_ACTIONS: [&str; 10] = [
    "scroll-up",
    "scroll-down",
    "scroll-left",
    "scroll-right",
    "scroll-page-up",
    "scroll-page-down",
    "scroll-half-page-up",
    "scroll-half-page-down",
    "scroll-start",
    "scroll-end",
];

/// The actions a keymap binds (SPEC §10.2): a text field's, `program`,
/// `prefix`, and the scroll actions.
pub const ACTIONS: [&str; 38] = [
    "char-backward",
    "char-forward",
    "word-backward",
    "word-forward",
    "line-start",
    "line-end",
    "delete-char-backward",
    "delete-char-forward",
    "delete-word-backward",
    "delete-word-forward",
    "delete-to-line-start",
    "delete-to-line-end",
    "line-previous",
    "line-next",
    "page-up",
    "page-down",
    "input-start",
    "input-end",
    "select-all",
    "newline",
    "submit",
    "program",
    "prefix",
    "copy",
    "cut",
    "paste",
    "undo",
    "redo",
    "scroll-up",
    "scroll-down",
    "scroll-left",
    "scroll-right",
    "scroll-page-up",
    "scroll-page-down",
    "scroll-half-page-up",
    "scroll-half-page-down",
    "scroll-start",
    "scroll-end",
];

/// The actions only a multi-line field has (SPEC §10.2). From an input, a
/// key bound to one reaches the program.
pub const MULTILINE_ACTIONS: [&str; 5] = [
    "line-previous",
    "line-next",
    "page-up",
    "page-down",
    "newline",
];

/// The moves, which select with Shift (SPEC §10.2).
pub const MOVES: [&str; 12] = [
    "char-backward",
    "char-forward",
    "word-backward",
    "word-forward",
    "line-start",
    "line-end",
    "line-previous",
    "line-next",
    "page-up",
    "page-down",
    "input-start",
    "input-end",
];

/// The edit actions, in SPEC §10.2's order: those a host's edit shortcuts
/// name ([`default_edit`]).
pub const EDIT_ACTIONS: [&str; 6] = ["select-all", "copy", "cut", "paste", "undo", "redo"];

/// What [`Keymap::lookup`] returns for a key the field types.
pub const INSERT: &str = "insert";

/// The SDK's keymap (SDK.md §3.10), the same in every SDK: the keys of
/// Bubble Tea's text input and text area, but `mod+a`, which selects all,
/// as a `data-keys` value. A program puts it in the `data-keys` of an
/// element that holds its fields, and gives its cells the same keymap,
/// `resolve(platform, multiline, [TERMINAL_KEYS])`. It leaves `enter` to
/// SPEC's default keymap.
pub const TERMINAL_KEYS: &str = concat!(
    "arrow_left=char-backward ctrl+b=char-backward arrow_right=char-forward ctrl+f=char-forward ",
    "alt+arrow_left=word-backward ctrl+arrow_left=word-backward alt+b=word-backward ",
    "alt+arrow_right=word-forward ctrl+arrow_right=word-forward alt+f=word-forward ",
    "home=line-start end=line-end ctrl+e=line-end ",
    "backspace=delete-char-backward ctrl+h=delete-char-backward ",
    "delete=delete-char-forward ctrl+d=delete-char-forward ",
    "alt+backspace=delete-word-backward ctrl+w=delete-word-backward ctrl+backspace=delete-word-backward ",
    "alt+delete=delete-word-forward alt+d=delete-word-forward ctrl+delete=delete-word-forward ",
    "ctrl+u=delete-to-line-start ctrl+k=delete-to-line-end ",
    "arrow_up=line-previous ctrl+p=line-previous arrow_down=line-next ctrl+n=line-next ",
    "page_up=page-up page_down=page-down ",
    "alt+<=input-start ctrl+home=input-start alt+>=input-end ctrl+end=input-end ",
    "mod+a=select-all ctrl+m=newline",
);

pub(crate) fn is_scroll(action: &str) -> bool {
    SCROLL_ACTIONS.contains(&action)
}

pub(crate) fn is_multiline(action: &str) -> bool {
    MULTILINE_ACTIONS.contains(&action)
}

pub(crate) fn is_move(action: &str) -> bool {
    MOVES.contains(&action)
}

const SUPER: u8 = 1;
const CTRL: u8 = 2;
const ALT: u8 = 4;
const SHIFT: u8 = 8;

/// The physical keys (myelin key names, Appendix A).
const PHYSICAL: &[&str] = &[
    "backquote",
    "backslash",
    "bracket_left",
    "bracket_right",
    "comma",
    "digit_0",
    "digit_1",
    "digit_2",
    "digit_3",
    "digit_4",
    "digit_5",
    "digit_6",
    "digit_7",
    "digit_8",
    "digit_9",
    "equal",
    "intl_backslash",
    "intl_ro",
    "intl_yen",
    "key_a",
    "key_b",
    "key_c",
    "key_d",
    "key_e",
    "key_f",
    "key_g",
    "key_h",
    "key_i",
    "key_j",
    "key_k",
    "key_l",
    "key_m",
    "key_n",
    "key_o",
    "key_p",
    "key_q",
    "key_r",
    "key_s",
    "key_t",
    "key_u",
    "key_v",
    "key_w",
    "key_x",
    "key_y",
    "key_z",
    "minus",
    "period",
    "quote",
    "semicolon",
    "slash",
    "alt_left",
    "alt_right",
    "backspace",
    "caps_lock",
    "context_menu",
    "control_left",
    "control_right",
    "enter",
    "meta_left",
    "meta_right",
    "shift_left",
    "shift_right",
    "space",
    "tab",
    "convert",
    "kana_mode",
    "non_convert",
    "delete",
    "end",
    "help",
    "home",
    "insert",
    "page_down",
    "page_up",
    "arrow_down",
    "arrow_left",
    "arrow_right",
    "arrow_up",
    "num_lock",
    "numpad_0",
    "numpad_1",
    "numpad_2",
    "numpad_3",
    "numpad_4",
    "numpad_5",
    "numpad_6",
    "numpad_7",
    "numpad_8",
    "numpad_9",
    "numpad_add",
    "numpad_backspace",
    "numpad_clear",
    "numpad_clear_entry",
    "numpad_comma",
    "numpad_decimal",
    "numpad_divide",
    "numpad_enter",
    "numpad_equal",
    "numpad_memory_add",
    "numpad_memory_clear",
    "numpad_memory_recall",
    "numpad_memory_store",
    "numpad_memory_subtract",
    "numpad_multiply",
    "numpad_paren_left",
    "numpad_paren_right",
    "numpad_subtract",
    "numpad_separator",
    "numpad_up",
    "numpad_down",
    "numpad_right",
    "numpad_left",
    "numpad_begin",
    "numpad_home",
    "numpad_end",
    "numpad_insert",
    "numpad_delete",
    "numpad_page_up",
    "numpad_page_down",
    "escape",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "f13",
    "f14",
    "f15",
    "f16",
    "f17",
    "f18",
    "f19",
    "f20",
    "f21",
    "f22",
    "f23",
    "f24",
    "f25",
    "fn",
    "fn_lock",
    "print_screen",
    "scroll_lock",
    "pause",
    "browser_back",
    "browser_favorites",
    "browser_forward",
    "browser_home",
    "browser_refresh",
    "browser_search",
    "browser_stop",
    "eject",
    "launch_app_1",
    "launch_app_2",
    "launch_mail",
    "media_play_pause",
    "media_select",
    "media_stop",
    "media_track_next",
    "media_track_previous",
    "power",
    "sleep",
    "audio_volume_down",
    "audio_volume_mute",
    "audio_volume_up",
    "wake_up",
    "copy",
    "cut",
    "paste",
];

/// Other keymaps' names for physical keys, which the default reading takes.
const OTHER: &[(&str, &str)] = &[
    ("up", "arrow_up"),
    ("down", "arrow_down"),
    ("left", "arrow_left"),
    ("right", "arrow_right"),
    ("pageup", "page_up"),
    ("pagedown", "page_down"),
    ("pgup", "page_up"),
    ("pgdn", "page_down"),
    ("esc", "escape"),
];

/// What a key's value is (myelin key names §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Character,
    Physical,
    Modifier,
}

/// A canonical key's name, split: its modifiers (a bit for each of
/// [`MODIFIERS`]) and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyParts {
    mods: u8,
    value: String,
}

impl KeyParts {
    /// A canonical key's parts.
    fn split(name: &str) -> KeyParts {
        let mut rest = name;
        let mut mods = 0;
        while let Some(i) = rest.find('+') {
            match MODIFIERS.iter().position(|m| *m == &rest[..i]) {
                Some(b) => mods |= 1 << b,
                None => break,
            }
            rest = &rest[i + 1..];
        }
        KeyParts {
            mods,
            value: rest.to_string(),
        }
    }

    fn shift(&self) -> bool {
        self.mods & SHIFT != 0
    }

    fn kind(&self) -> Kind {
        if PHYSICAL.contains(&self.value.as_str()) {
            Kind::Physical
        } else if MODIFIERS.contains(&self.value.as_str()) {
            Kind::Modifier
        } else {
            Kind::Character
        }
    }

    fn name(&self) -> String {
        join(self.mods, &self.value)
    }

    fn with(mut self, mods: u8) -> KeyParts {
        self.mods |= mods;
        self
    }
}

fn join(mods: u8, value: &str) -> String {
    let mut out = String::new();
    for (i, m) in MODIFIERS.iter().enumerate() {
        if mods & (1 << i) != 0 {
            out.push_str(m);
            out.push('+');
        }
    }
    out.push_str(value);
    out
}

/// The one char a case mapping gives, if it gives one.
fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

/// Each code point's simple lowercase mapping.
fn lower(s: &str) -> String {
    s.chars()
        .map(|c| single(c.to_lowercase()).unwrap_or(c))
        .collect()
}

/// Each code point's simple uppercase mapping.
fn upper(s: &str) -> String {
    s.chars()
        .map(|c| single(c.to_uppercase()).unwrap_or(c))
        .collect()
}

fn is_capital(c: &str) -> bool {
    c.chars().count() == 1 && lower(c) != c && upper(&lower(c)) == c
}

/// Whether a character has case: its simple uppercase mapping differs, or
/// it is a capital.
fn is_letter(c: &str) -> bool {
    upper(c) != c || is_capital(c)
}

/// Whether a key's value is a character (myelin key names §2.4): one
/// grapheme cluster, its first code point neither a space nor a control.
fn is_char(v: &str) -> bool {
    let mut g = v.graphemes(true);
    match (g.next(), g.next()) {
        (Some(first), None) => !first
            .chars()
            .next()
            .is_some_and(|c| c == ' ' || c.is_control()),
        _ => false,
    }
}

/// A physical key as the default reading takes it: its name in any case,
/// another keymap's name for it, or a W3C code value (`PageDown`).
fn physical(part: &str) -> Option<String> {
    let low = part.to_lowercase();
    if PHYSICAL.contains(&low.as_str()) {
        return Some(low);
    }
    if let Some((_, p)) = OTHER.iter().find(|(o, _)| *o == low) {
        return Some(p.to_string());
    }
    let mut out = String::new();
    for (i, c) in part.chars().enumerate() {
        if !c.is_ascii_alphanumeric() {
            return None;
        }
        if i > 0 && (c.is_ascii_uppercase() || c.is_ascii_digit()) {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    PHYSICAL.contains(&out.as_str()).then_some(out)
}

/// A key's name, read with myelin key names' default reading as SPEC §10.4
/// has hosts read one, in its canonical form: `Shift+Ctrl+A` is
/// `ctrl+shift+a`, `PageDown` is `page_down`, `esc` is `escape`. `None`
/// where the reading refuses it (empty, `meta+a`, `ctrl+`, a modifier given
/// twice), and for a name with `mod`, which only a [`Platform`] reads
/// ([`read_key`]).
pub fn parse_key(name: &str) -> Option<String> {
    read(name, None, false)
}

/// A key's name read as [`parse_key`] reads it, with `mod` standing for the
/// platform's modifier (myelin key names §4.4): on `ctrl`, `mod+s` is
/// `ctrl+s`, and `mod+ctrl+s` is `None`.
pub fn read_key(platform: &Platform, name: &str) -> Option<String> {
    read(name, Some(platform.modifier), false)
}

/// The default reading of a key's name, `mod` standing for the modifier
/// given (refused when `None`). With `keep`, the key keeps `mod`, first,
/// where it was written with it.
fn read(text: &str, modifier: Option<Mod>, keep: bool) -> Option<String> {
    if text.is_empty() || is_alias(text) {
        return None;
    }
    let mut parts = Vec::new();
    let mut i = 0;
    loop {
        match text[i..].find('+') {
            None => {
                parts.push(&text[i..]);
                break;
            }
            Some(j) => {
                parts.push(&text[i..i + j]);
                i += j + 1;
                if i == text.len() {
                    if parts.last().is_some_and(|p| !p.is_empty()) {
                        return None; // `ctrl+`, `a+`
                    }
                    break;
                }
            }
        }
    }
    let (mut mods, mut written, mut named) = (0u8, 0u8, 0u8);
    let mut order = Vec::new();
    let mut value: Option<String> = None;
    let mut mod_named = false;
    for p in parts {
        let low = p.to_lowercase();
        if low == "meta" {
            return None;
        }
        let mut m = match low.as_str() {
            "super" | "cmd" | "command" => Some(SUPER),
            "ctrl" | "control" => Some(CTRL),
            "alt" | "opt" | "option" => Some(ALT),
            "shift" => Some(SHIFT),
            _ => None,
        };
        if low == "mod" {
            let bit = match modifier? {
                Mod::Super => SUPER,
                Mod::Ctrl => CTRL,
            };
            // `mod` with `super` or `ctrl` is refused (SPEC §10.4), and
            // `mod` twice, as any modifier is.
            if mod_named || named & (SUPER | CTRL) != 0 {
                return None;
            }
            mod_named = true;
            written |= bit;
            m = Some(bit);
        } else if let Some(bit) = m {
            if bit & (SUPER | CTRL) != 0 && mod_named {
                return None;
            }
            named |= bit;
        }
        if let Some(bit) = m {
            if mods & bit != 0 {
                return None;
            }
            mods |= bit;
            order.push(bit);
            continue;
        }
        if value.is_some() {
            return None;
        }
        value = Some(if p.is_empty() {
            "+".to_string()
        } else if is_char(p) {
            lower(p)
        } else {
            physical(p)?
        });
    }
    let value = match value {
        Some(v) => v,
        None => {
            // A modifier tapped on its own: the last one is the key.
            let last = *order.last()?;
            mods &= !last;
            written &= !last;
            MODIFIERS[last.trailing_zeros() as usize].to_string()
        }
    };
    if keep && written != 0 {
        Some(format!("mod+{}", join(mods & !written, &value)))
    } else {
        Some(join(mods, &value))
    }
}

/// A name like `<c-a>`, which the default reading refuses.
fn is_alias(text: &str) -> bool {
    text.len() > 2
        && text.starts_with('<')
        && text.ends_with('>')
        && text[1..text.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The text a key types in a field (SPEC §10.2): a character's, a letter
/// with `shift` as its capital (`shift+a` types `A`), a space for `space`;
/// `None` for any other key, or for a key with `ctrl`, `alt` or `super`.
pub fn key_text(key: &str) -> Option<String> {
    let k = KeyParts::split(&parse_key(key)?);
    if k.mods & (SUPER | CTRL | ALT) != 0 {
        return None;
    }
    if k.value == "space" {
        return Some(" ".into());
    }
    if k.kind() != Kind::Character {
        return None;
    }
    Some(if k.shift() && is_letter(&k.value) {
        upper(&k.value)
    } else {
        k.value
    })
}

/// The key of a character with modifiers: a capital is `shift` with its
/// letter, the space bar is `space`, and `shift` is in any other character
/// already (`!`, not `shift+!`).
fn char_key(mut mods: u8, c: &str) -> String {
    if mods & SHIFT != 0 && !is_letter(c) && c != " " {
        mods &= !SHIFT;
    }
    if c == " " {
        return join(mods, "space");
    }
    if is_capital(c) {
        return join(mods | SHIFT, &lower(c));
    }
    join(mods, c)
}

/// The keys in input from the terminal, the bytes a program reads, as SPEC
/// §10.4 names them: one entry for each key, its canonical name, or `None`
/// for input that is no key it names (a mouse report, a sequence it does
/// not know, a key's release). It is pure: an ESC at the end of the input
/// is `escape`, and the caller passes each read whole.
pub fn decode_keys(input: &[u8]) -> Vec<Option<String>> {
    decode_key_spans(input)
        .into_iter()
        .map(|(_, k)| k)
        .collect()
}

/// [`decode_keys`], with the bytes each key took: for a program that uses
/// some of the keys and passes the others on as they came.
pub fn decode_key_spans(input: &[u8]) -> Vec<(Range<usize>, Option<String>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let at = i;
        let key = if input[i] != 0x1b {
            let (k, n) = one_key(&input[i..]);
            i += n;
            k
        } else if i + 1 == input.len() {
            i += 1;
            Some("escape".into())
        } else if input[i + 1] == b'O' && i + 2 < input.len() {
            i += 3;
            final_key(input[i - 1]).map(str::to_string)
        } else if input[i + 1] == b'[' && i + 2 < input.len() {
            let mut j = i + 2;
            while j < input.len() && (0x30..=0x3f).contains(&input[j]) {
                j += 1;
            }
            let params = std::str::from_utf8(&input[i + 2..j]).unwrap_or("");
            while j < input.len() && (0x20..=0x2f).contains(&input[j]) {
                j += 1;
            }
            i = (j + 1).min(input.len());
            if j == input.len() || !(0x40..=0x7e).contains(&input[j]) {
                None
            } else {
                csi_key(params, input[j])
            }
        } else {
            let (k, n) = one_key(&input[i + 1..]);
            i += 1 + n;
            k.map(|k| KeyParts::split(&k).with(ALT).name())
        };
        out.push((at..i, key));
    }
    out
}

/// The key of the control or the character the input starts with, and its
/// length in bytes.
fn one_key(input: &[u8]) -> (Option<String>, usize) {
    let b = input[0];
    let name = match b {
        0x00 => Some("ctrl+space"),
        0x08 => Some("ctrl+h"),
        0x09 => Some("tab"),
        0x0d => Some("enter"),
        0x1b => Some("escape"),
        0x7f => Some("backspace"),
        0x1c => Some("ctrl+\\"),
        0x1d => Some("ctrl+]"),
        0x1e => Some("ctrl+^"),
        0x1f => Some("ctrl+_"),
        0x20 => Some("space"),
        _ => None,
    };
    if let Some(n) = name {
        return (Some(n.into()), 1);
    }
    if b < 0x20 {
        return (Some(format!("ctrl+{}", (b + 0x60) as char)), 1);
    }
    let end = input.len().min(64);
    let text = match std::str::from_utf8(&input[..end]) {
        Ok(t) => t,
        Err(e) if e.valid_up_to() > 0 => {
            std::str::from_utf8(&input[..e.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return (None, 1),
    };
    let g = text.graphemes(true).next().unwrap_or("");
    // A cluster stops at a control, which is a key of its own.
    let g = match g.char_indices().skip(1).find(|(_, c)| c.is_control()) {
        Some((i, _)) => &g[..i],
        None => g,
    };
    (Some(char_key(0, g)), g.len().max(1))
}

/// The keys CSI and SS3 name by their final byte.
fn final_key(f: u8) -> Option<&'static str> {
    Some(match f {
        b'A' => "arrow_up",
        b'B' => "arrow_down",
        b'C' => "arrow_right",
        b'D' => "arrow_left",
        b'H' => "home",
        b'F' => "end",
        _ => return None,
    })
}

/// A CSI parameter `m[:e]`: its modifiers, and whether it is a release.
fn mods_of(field: &str) -> Option<(u8, bool)> {
    let (num, ev) = field.split_once(':').unwrap_or((field, ""));
    let n: i64 = if num.is_empty() { 1 } else { num.parse().ok()? };
    let e: i64 = if ev.is_empty() { 1 } else { ev.parse().ok()? };
    let bits = (n - 1).max(0);
    let mut m = 0;
    if bits & 1 != 0 {
        m |= SHIFT;
    }
    if bits & 2 != 0 {
        m |= ALT;
    }
    if bits & 4 != 0 {
        m |= CTRL;
    }
    if bits & (8 | 32) != 0 {
        m |= SUPER;
    }
    Some((m, e == 3))
}

/// The key a code of the kitty keyboard protocol or of modifyOtherKeys names.
fn code_key(code: u32, mods: u8, shifted: u32, text: &str) -> Option<String> {
    let named = match code {
        9 => "tab",
        13 => "enter",
        27 => "escape",
        8 | 127 => "backspace",
        32 => "space",
        57399..=57408 => {
            return Some(join(
                mods,
                &char::from(b'0' + (code - 57399) as u8).to_string(),
            ));
        }
        57409 => return Some(join(mods, ".")),
        57410 => return Some(join(mods, "/")),
        57411 => return Some(join(mods, "*")),
        57412 => return Some(join(mods, "-")),
        57413 => return Some(join(mods, "+")),
        57414 => "enter",
        57415 => return Some(join(mods, "=")),
        57417 => "arrow_left",
        57418 => "arrow_right",
        57419 => "arrow_up",
        57420 => "arrow_down",
        57421 => "page_up",
        57422 => "page_down",
        57423 => "home",
        57424 => "end",
        57425 => "insert",
        57426 => "delete",
        57344.. => return None,
        _ => "",
    };
    if !named.is_empty() {
        return Some(join(mods, named));
    }
    if code < 0x20 || code == 0x7f {
        return None;
    }
    let base = char::from_u32(code)?.to_string();
    if is_letter(&base) {
        return Some(join(mods, &lower(&base)));
    }
    if mods & SHIFT != 0 {
        if let Some(s) = char::from_u32(shifted).filter(|_| shifted > 0) {
            return Some(char_key(mods & !SHIFT, &s.to_string()));
        }
        if !text.is_empty() {
            return Some(char_key(mods & !SHIFT, text));
        }
    }
    Some(join(mods, &base))
}

/// The key a CSI sequence names.
fn csi_key(params: &str, fin: u8) -> Option<String> {
    if params.starts_with(['<', '=', '>', '?']) {
        return None;
    }
    let fields: Vec<&str> = params.split(';').collect();
    let field = |i: usize| fields.get(i).copied().unwrap_or("");
    match fin {
        b'u' => {
            let mut codes = field(0).split(':');
            let code: u32 = codes.next()?.parse().ok()?;
            let shifted: u32 = match codes.next() {
                Some(s) if !s.is_empty() => s.parse().ok()?,
                _ => 0,
            };
            let (mods, release) = mods_of(field(1))?;
            if release {
                return None;
            }
            let mut text = String::new();
            if !field(2).is_empty() {
                for c in field(2).split(':') {
                    text.push(char::from_u32(c.parse().ok()?)?);
                }
            }
            code_key(code, mods, shifted, &text)
        }
        b'~' => {
            let n: u32 = field(0).parse().ok()?;
            if n == 27 && fields.len() >= 3 {
                let (mods, release) = mods_of(fields[1])?;
                if release {
                    return None;
                }
                return code_key(fields[2].parse().ok()?, mods, 0, "");
            }
            let v = match n {
                1 | 7 => "home",
                4 | 8 => "end",
                2 => "insert",
                3 => "delete",
                5 => "page_up",
                6 => "page_down",
                _ => return None,
            };
            let (mods, release) = mods_of(field(1))?;
            (!release).then(|| join(mods, v))
        }
        b'Z' => Some("shift+tab".into()),
        f => {
            let v = final_key(f)?;
            let (mods, release) = mods_of(field(1))?;
            (!release).then(|| join(mods, v))
        }
    }
}

/// What `mod` stands for in bindings (SPEC §10.4): the capabilities' `mod`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mod {
    /// `ctrl`, where a host says nothing: Linux and Windows.
    #[default]
    Ctrl,
    /// `super`: Command, on macOS.
    Super,
}

impl Mod {
    /// The modifier's name, as the capabilities write it: `"ctrl"` or `"super"`.
    pub fn name(self) -> &'static str {
        match self {
            Mod::Ctrl => "ctrl",
            Mod::Super => "super",
        }
    }
}

/// How a host reads bindings (SDK.md §3.10): what `mod` stands for, and its
/// edit shortcuts. The default Platform is `ctrl` with the default edit
/// shortcuts. Canonically `Platform{Mod, Edit}`; `mod` is a Rust keyword.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Platform {
    /// What `mod` stands for.
    pub modifier: Mod,
    /// The edit shortcuts, from each action to its keys (SPEC §10.2); `None`
    /// is [`default_edit`] for the modifier.
    pub edit: Option<Vec<(String, Vec<String>)>>,
}

impl Platform {
    /// The Platform of a modifier, with its default edit shortcuts.
    pub fn new(modifier: Mod) -> Platform {
        Platform {
            modifier,
            edit: None,
        }
    }

    /// The edit shortcuts: the platform's, or [`default_edit`] for its modifier.
    pub fn edit_shortcuts(&self) -> Vec<(String, Vec<String>)> {
        self.edit
            .clone()
            .unwrap_or_else(|| default_edit(self.modifier))
    }
}

/// The Platform of a host's capabilities (SDK.md §3.10): its `mod` and its
/// edit shortcuts, each the default Platform's when absent. A `mod` other
/// than `"super"` is `ctrl`.
pub fn platform_of(caps: &Caps) -> Platform {
    Platform {
        modifier: match caps.modifier.as_deref() {
            Some("super") => Mod::Super,
            _ => Mod::Ctrl,
        },
        edit: caps.edit.clone(),
    }
}

/// The default edit shortcuts for a `mod` (SPEC §10.2), in its order, each
/// key read: `ctrl+shift+z` and `ctrl+y` for `redo` where `mod` is `ctrl`.
pub fn default_edit(modifier: Mod) -> Vec<(String, Vec<String>)> {
    let m = modifier.name();
    EDIT_ACTIONS
        .iter()
        .zip(["a", "c", "x", "v", "z", "shift+z"])
        .map(|(a, k)| {
            let mut keys = vec![format!("{m}+{k}")];
            if *a == "redo" && modifier == Mod::Ctrl {
                keys.push("ctrl+y".into());
            }
            (a.to_string(), keys)
        })
        .collect()
}

/// A set of bindings, key to action, read on a [`Platform`] (SDK.md
/// §3.10, SPEC §10.2). [`parse_keymap`] reads one from a `data-keys`
/// value, and [`resolve`] builds a text field's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    /// Each key, canonical with `mod` read, in the order first bound, and
    /// its action.
    bindings: Vec<(String, String)>,
    /// Each key's last binding as written, `mod` kept, by index.
    written: Vec<String>,
    multiline: bool,
    modifier: Mod,
}

/// Reads a `data-keys` value as SPEC §10.2 has hosts read it: bindings
/// separated by ASCII white space, each `key=action`, split at its last
/// `=`, each key read with [`read_key`]; dropping a key that does not
/// parse, an action it does not know, and `tab`, `shift+tab` and `escape`.
/// On `ctrl`, `mod+a` and `ctrl+a` are one key, the later binding's.
///
/// An element that is not a text field has the keymap of its `data-keys`
/// values joined with a space, the root's first, with no default keymap:
/// what [`Keymap::program`], [`Keymap::prefix`] and [`Keymap::scroll`] ask.
pub fn parse_keymap(platform: &Platform, value: &str) -> Keymap {
    let mut m = Keymap::new(platform.modifier, false);
    for (k, a) in bindings(value) {
        m.bind(k, a);
    }
    m
}

/// A `data-keys` value's bindings as written, in order, each split at its
/// last `=`.
fn bindings(value: &str) -> impl Iterator<Item = (&str, &str)> {
    value
        .split([' ', '\t', '\n', '\x0c', '\r'])
        .filter_map(|b| b.rfind('=').map(|i| (&b[..i], &b[i + 1..])))
}

/// The default keymap's keys on every host (SPEC §10.2); `enter`, those of
/// the platform's `mod` and the edit shortcuts follow.
const DEFAULT_KEYS: &[(&str, &str)] = &[
    ("arrow_left", "char-backward"),
    ("arrow_right", "char-forward"),
    ("alt+arrow_left", "word-backward"),
    ("alt+arrow_right", "word-forward"),
    ("home", "line-start"),
    ("end", "line-end"),
    ("backspace", "delete-char-backward"),
    ("delete", "delete-char-forward"),
    ("alt+backspace", "delete-word-backward"),
    ("alt+delete", "delete-word-forward"),
    ("arrow_up", "line-previous"),
    ("arrow_down", "line-next"),
    ("page_up", "page-up"),
    ("page_down", "page-down"),
];

/// The default keymap's keys where `mod` is `ctrl`.
const CTRL_KEYS: &[(&str, &str)] = &[
    ("ctrl+arrow_left", "word-backward"),
    ("ctrl+arrow_right", "word-forward"),
    ("ctrl+home", "input-start"),
    ("ctrl+end", "input-end"),
    ("ctrl+backspace", "delete-word-backward"),
    ("ctrl+delete", "delete-word-forward"),
];

/// The default keymap's keys where `mod` is `super`: macOS's, with Cocoa's
/// `ctrl` keys.
const SUPER_KEYS: &[(&str, &str)] = &[
    ("super+arrow_left", "line-start"),
    ("super+arrow_right", "line-end"),
    ("super+arrow_up", "input-start"),
    ("super+arrow_down", "input-end"),
    ("super+backspace", "delete-to-line-start"),
    ("ctrl+a", "line-start"),
    ("ctrl+e", "line-end"),
    ("ctrl+b", "char-backward"),
    ("ctrl+f", "char-forward"),
    ("ctrl+p", "line-previous"),
    ("ctrl+n", "line-next"),
    ("ctrl+h", "delete-char-backward"),
    ("ctrl+d", "delete-char-forward"),
    ("ctrl+k", "delete-to-line-end"),
];

/// A text field's keymap (SDK.md §3.10): SPEC §10.2's default keymap for
/// the platform's `mod`, with its edit shortcuts, for an `input` or for a
/// multi-line field, then each `data-keys` value in turn, the root's first.
/// A binding to a scroll action is left out where it stands, in its own
/// value too: it neither acts nor overrides an earlier binding of its key.
pub fn resolve<'a>(
    platform: &Platform,
    multiline: bool,
    values: impl IntoIterator<Item = &'a str>,
) -> Keymap {
    let mut m = Keymap::new(platform.modifier, multiline);
    for (k, a) in DEFAULT_KEYS {
        m.bind(k, a);
    }
    m.bind("enter", if multiline { "newline" } else { "submit" });
    let own = match platform.modifier {
        Mod::Ctrl => CTRL_KEYS,
        Mod::Super => SUPER_KEYS,
    };
    for (k, a) in own {
        m.bind(k, a);
    }
    for (a, keys) in platform.edit_shortcuts() {
        for k in keys {
            m.bind(&k, &a);
        }
    }
    for v in values {
        for (k, a) in bindings(v).filter(|(_, a)| !is_scroll(a)) {
            m.bind(k, a);
        }
    }
    m
}

impl Keymap {
    fn new(modifier: Mod, multiline: bool) -> Keymap {
        Keymap {
            multiline,
            modifier,
            ..Keymap::default()
        }
    }

    /// Binds a key to an action, as a later binding in a `data-keys` value
    /// does; false, binding nothing, where a host ignores the binding (a
    /// key that does not parse, an action it does not know, `tab`,
    /// `shift+tab` or `escape`).
    pub fn bind(&mut self, key: &str, action: &str) -> bool {
        let (Some(k), Some(w)) = (
            read(key, Some(self.modifier), false),
            read(key, Some(self.modifier), true),
        ) else {
            return false;
        };
        if !ACTIONS.contains(&action) || matches!(k.as_str(), "tab" | "shift+tab" | "escape") {
            return false;
        }
        match self.bindings.iter().position(|(b, _)| *b == k) {
            Some(i) => {
                self.bindings[i].1 = action.to_string();
                self.written[i] = w;
            }
            None => {
                self.bindings.push((k, action.to_string()));
                self.written.push(w);
            }
        }
        true
    }

    /// The bindings, each key canonical with `mod` read, in the order first
    /// bound, with its last action.
    pub fn bindings(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.bindings.iter().map(|(k, a)| (k.as_str(), a.as_str()))
    }

    /// The keymap as a `data-keys` value: each key once, where it was first
    /// bound, as its last binding wrote it, with its last action, as
    /// `key=action` separated by a space. A key is written canonical, and
    /// one written with `mod` keeps it, first (`mod+shift+z`).
    pub fn format(&self) -> String {
        self.bindings
            .iter()
            .zip(&self.written)
            .map(|((_, a), w)| format!("{w}={a}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, a)| a.as_str())
    }

    /// The action bound to a canonical key, or, for a key with `shift`
    /// that is not a character and that is not bound itself, to the key
    /// without `shift` (SPEC §10.2).
    fn bound(&self, key: &str) -> Option<&str> {
        self.get(key).or_else(|| {
            let k = KeyParts::split(key);
            if !k.shift() || k.kind() == Kind::Character {
                return None;
            }
            self.get(&join(k.mods & !SHIFT, &k.value))
        })
    }

    /// Whether the keymap binds a key, or, for a key with `shift` that is
    /// not a character and that it does not bind, the key without `shift`
    /// (SPEC §10.2). A host that has two names for a press (SPEC §10.4) asks
    /// what follows of the physical one where the keymap binds it, and of
    /// the other one otherwise.
    pub fn binds(&self, key: &str) -> bool {
        parse_key(key).is_some_and(|k| self.bound(&k).is_some())
    }

    /// What a field with this keymap does with a key: an action; [`INSERT`]
    /// for a key it types ([`key_text`]); or `None` when the key is not the
    /// field's, and reaches the program, or moves focus. `tab`, `shift+tab`
    /// and `escape` are never the field's, nor a key bound to `program` or
    /// `prefix`, or to an action the field does not have (a multi-line one
    /// in an input).
    pub fn lookup(&self, key: &str) -> Option<&str> {
        let k = parse_key(key)?;
        if matches!(k.as_str(), "tab" | "shift+tab" | "escape") {
            return None;
        }
        match self.bound(&k) {
            Some("program" | "prefix") => None,
            Some(a) if is_multiline(a) && !self.multiline => None,
            Some(a) => Some(a),
            None => key_text(&k).map(|_| INSERT),
        }
    }

    /// Whether a field with this keymap selects with a key (SPEC §10.2,
    /// Shift selects): [`Keymap::lookup`] returns a move for it, and its
    /// name has `shift` (`shift+arrow_left`, `ctrl+shift+end`, not
    /// `alt+<`). The field then does the move with
    /// [`Field::extend`](crate::Field::extend).
    pub fn selects(&self, key: &str) -> bool {
        parse_key(key).is_some_and(|k| KeyParts::split(&k).shift())
            && self.lookup(key).is_some_and(is_move)
    }

    /// Whether the keymap gives a key to the program (SPEC §10.2, keys for
    /// the program): it binds the key to `program` or `prefix`, with the
    /// fallback of [`Keymap::binds`]. A host asks it of a focused element's
    /// keymap, and the key reaches the program before the element or a
    /// scroll uses it.
    pub fn program(&self, key: &str) -> bool {
        parse_key(key).is_some_and(|k| matches!(self.bound(&k), Some("program" | "prefix")))
    }

    /// Whether the keymap binds a key to `prefix`, with the same fallback:
    /// the key reaches the program, and so does every key after it, until
    /// the program sends `a=focus` (SPEC §10.5).
    pub fn prefix(&self, key: &str) -> bool {
        parse_key(key).is_some_and(|k| self.bound(&k) == Some("prefix"))
    }

    /// The scroll action the keymap binds a key to, with the same fallback,
    /// or `None` (SPEC §10.2, scrolling keys). A host asks it of an
    /// element's keymap for a key the element does not use.
    pub fn scroll(&self, key: &str) -> Option<&str> {
        self.bound(&parse_key(key)?).filter(|a| is_scroll(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platform_reads_mod_as_its_own() {
        let mac = Platform::new(Mod::Super);
        assert_eq!(read_key(&mac, "mod+s").as_deref(), Some("super+s"));
        assert_eq!(
            read_key(&Platform::default(), "mod+s").as_deref(),
            Some("ctrl+s")
        );
        assert_eq!(read_key(&Platform::default(), "mod+ctrl+s"), None);
        assert_eq!(parse_key("mod+s"), None);
    }

    #[test]
    fn platform_of_takes_the_host_s_mod_and_edit() {
        let caps = Caps {
            modifier: Some("super".into()),
            ..Caps::default()
        };
        let p = platform_of(&caps);
        assert_eq!(p.modifier, Mod::Super);
        assert_eq!(
            p.edit_shortcuts()[0],
            ("select-all".into(), vec!["super+a".into()])
        );
        assert_eq!(platform_of(&Caps::default()), Platform::default());
    }

    #[test]
    fn key_spans_cover_the_input() {
        let s = decode_key_spans(b"a\x1b[Ab");
        assert_eq!(
            s,
            vec![
                (0..1, Some("a".into())),
                (1..4, Some("arrow_up".into())),
                (4..5, Some("b".into())),
            ]
        );
    }
}
