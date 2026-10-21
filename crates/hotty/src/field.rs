//! A text field's value, caret and selection, edited as SPEC §10.2's
//! actions edit them, for a program that draws its fields in cells (SDK.md
//! §4.6, the second rendition).
//!
//! ```
//! use hotty::{Field, INSERT, Platform, TERMINAL_KEYS, resolve};
//!
//! let keys = resolve(&Platform::default(), false, [TERMINAL_KEYS]);
//! let mut f = Field::new("foo bar");
//! let (action, changed) = f.key(&keys, "ctrl+w");
//! assert_eq!((action, changed), (Some("delete-word-backward"), true));
//! assert_eq!(f.value, "foo ");
//! assert_eq!(f.key(&keys, "shift+a"), (Some(INSERT), true));
//! assert_eq!(f.value, "foo A");
//! ```
//!
//! Rows are lines: a field in cells does not wrap, and the place along a
//! row is a count of characters. Characters are grapheme clusters (UAX
//! #29), CR LF one. A rendition that wraps moves the caret between its rows
//! itself.

use unicode_segmentation::UnicodeSegmentation;

use crate::keys::{INSERT, Keymap, is_move, is_multiline, key_text};

/// A text field's value, caret and selection (SDK.md §4.6). The selection
/// runs from the anchor, the end where it began, to the caret; nothing is
/// selected when the anchor is `None` or at the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The value.
    pub value: String,
    /// The caret, in characters (grapheme clusters) from the start.
    pub caret: usize,
    /// Where the selection began, in characters; `None` when nothing is
    /// selected.
    pub anchor: Option<usize>,
    /// Whether the field is multi-line, a `textarea`: it has the multi-line
    /// actions, and its lines are rows.
    pub multiline: bool,
    /// Whether the field is a password: one word (SPEC §10.2).
    pub password: bool,
    /// The rows the field shows, for `page-up` and `page-down`; 0 is 1.
    pub rows: usize,
    /// The place along the row a run of row moves keeps.
    goal: Option<usize>,
}

impl Default for Field {
    fn default() -> Field {
        Field {
            value: String::new(),
            caret: 0,
            anchor: None,
            multiline: false,
            password: false,
            rows: 1,
            goal: None,
        }
    }
}

/// The moves that go back or up, which start from a selection's start.
const BACK: [&str; 6] = [
    "char-backward",
    "word-backward",
    "line-start",
    "line-previous",
    "page-up",
    "input-start",
];

const ROW_MOVES: [&str; 4] = ["line-previous", "line-next", "page-up", "page-down"];

impl Field {
    /// A one-line field holding `value`, its caret at the end.
    pub fn new(value: impl Into<String>) -> Field {
        let value = value.into();
        Field {
            caret: chars(&value).len(),
            value,
            ..Field::default()
        }
    }

    /// The selection's start and end, in characters, equal when nothing is
    /// selected: what the rendition draws as selected.
    pub fn selection(&self) -> (usize, usize) {
        let n = chars(&self.value).len();
        let p = self.caret.min(n);
        let a = self.anchor.map_or(p, |a| a.min(n));
        (a.min(p), a.max(p))
    }

    /// Selects from `anchor` to `caret`, as the user does with the pointer;
    /// `select(p, p)` puts the caret at `p` with nothing selected. It ends a
    /// run of row moves.
    pub fn select(&mut self, anchor: usize, caret: usize) {
        self.goal = None;
        self.caret = caret;
        self.anchor = (anchor != caret).then_some(anchor);
    }

    /// Does an action, and returns whether the value changed: a program
    /// reports an `input` when it did. A selection comes first (SPEC
    /// §10.2): a delete deletes it and nothing else; a move starts from its
    /// start when it goes back or up, and from its end otherwise, and ends
    /// it, `char-backward` and `char-forward` stopping there; `select-all`
    /// selects the whole value. `submit` and `program` are the program's,
    /// and so are `copy`, `cut`, `paste`, `undo` and `redo` (a Field keeps
    /// no clipboard and no history); they, and an action the field does not
    /// have (a multi-line one in an input, a scroll action), change nothing.
    ///
    /// Canonically `Do`; `do` is a Rust keyword.
    pub fn do_action(&mut self, action: &str) -> bool {
        self.act(action, false)
    }

    /// Does a move as Shift does it (SPEC §10.2): the anchor stays, or is
    /// set where the caret is, and the caret moves from where it is. An
    /// action that is not a move is [`Field::do_action`]'s.
    pub fn extend(&mut self, action: &str) -> bool {
        self.act(action, is_move(action))
    }

    /// Types text in place of the selection, or at the caret, and returns
    /// whether the value changed. It ends a run of row moves.
    ///
    /// Canonically `Type`; `type` is a Rust keyword.
    pub fn type_text(&mut self, text: &str) -> bool {
        self.goal = None;
        if text.is_empty() {
            return false;
        }
        let c = chars(&self.value);
        let (lo, hi) = self.selection();
        self.anchor = None;
        let before = c[..lo].concat() + text;
        self.caret = chars(&before).len();
        self.value = before + &c[hi..].concat();
        true
    }

    /// Does what the keymap says of a key (SDK.md §4.6): types its
    /// [`key_text`] where [`Keymap::lookup`] says [`INSERT`], extends with
    /// it where [`Keymap::selects`], and does its action otherwise. Returns
    /// the action, `None` when the key is not the field's (it is the
    /// program's, or `tab` moves focus), and whether the value changed.
    pub fn key<'k>(&mut self, keymap: &'k Keymap, key: &str) -> (Option<&'k str>, bool) {
        let Some(a) = keymap.lookup(key) else {
            return (None, false);
        };
        if a == INSERT {
            let text = key_text(key).unwrap_or_default();
            return (Some(a), self.type_text(&text));
        }
        if keymap.selects(key) {
            return (Some(a), self.extend(a));
        }
        (Some(a), self.do_action(a))
    }

    fn act(&mut self, action: &str, extend: bool) -> bool {
        let value = self.value.clone();
        let c = chars(&value);
        let n = c.len();
        let mut p = self.caret.min(n);
        if !ROW_MOVES.contains(&action) {
            self.goal = None;
        }
        if is_multiline(action) && !self.multiline {
            return false;
        }
        let (lo, hi) = self.selection();
        if action == "select-all" {
            self.anchor = Some(0);
            self.caret = n;
            return false;
        }
        if action == "newline" {
            return self.type_text("\n");
        }
        if extend {
            self.anchor.get_or_insert(p);
        } else if lo < hi && is_move(action) {
            self.anchor = None;
            p = if BACK.contains(&action) { lo } else { hi };
            if action == "char-backward" || action == "char-forward" {
                self.caret = p;
                return false;
            }
        } else if lo < hi && action.starts_with("delete-") {
            self.anchor = None;
            return self.delete(&c, lo, lo, hi);
        } else if lo == hi {
            self.anchor = None;
        }
        let (s, e) = self.line(&c, p);
        let page = self.rows.max(1) as isize;
        let to = match action {
            "char-backward" => Some(p.saturating_sub(1)),
            "char-forward" => Some((p + 1).min(n)),
            "word-backward" => Some(self.word_backward(&c, p)),
            "word-forward" => Some(self.word_forward(&c, p)),
            "line-start" => Some(s),
            "line-end" => Some(e),
            "line-previous" => Some(self.row(&c, p, -1)),
            "line-next" => Some(self.row(&c, p, 1)),
            "page-up" => Some(self.row(&c, p, -page)),
            "page-down" => Some(self.row(&c, p, page)),
            "input-start" => Some(0),
            "input-end" => Some(n),
            _ => None,
        };
        if let Some(to) = to {
            self.caret = to;
            if self.anchor == Some(to) {
                self.anchor = None;
            }
            return false;
        }
        match action {
            "delete-char-backward" => self.delete(&c, p, p.saturating_sub(1), p),
            "delete-char-forward" => self.delete(&c, p, p, (p + 1).min(n)),
            "delete-word-backward" => self.delete(&c, p, self.word_backward(&c, p), p),
            "delete-word-forward" => self.delete(&c, p, p, self.word_forward(&c, p)),
            "delete-to-line-start" => self.delete(&c, p, s, p),
            "delete-to-line-end" => self.delete(&c, p, p, e),
            _ => false,
        }
    }

    /// The start and the end of each line; an input's value is one.
    fn lines(&self, c: &[&str]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        if self.multiline {
            for (i, ch) in c.iter().enumerate() {
                if is_break(ch) {
                    out.push((start, i));
                    start = i + 1;
                }
            }
        }
        out.push((start, c.len()));
        out
    }

    /// The line the caret at `p` is on.
    fn line(&self, c: &[&str], p: usize) -> (usize, usize) {
        self.lines(c)
            .into_iter()
            .find(|&(s, e)| s <= p && p <= e)
            .unwrap_or((0, c.len()))
    }

    fn word_backward(&self, c: &[&str], mut p: usize) -> usize {
        if self.password {
            return 0;
        }
        while p > 0 && is_space(c[p - 1]) {
            p -= 1;
        }
        while p > 0 && !is_space(c[p - 1]) {
            p -= 1;
        }
        p
    }

    fn word_forward(&self, c: &[&str], mut p: usize) -> usize {
        if self.password {
            return c.len();
        }
        while p < c.len() && is_space(c[p]) {
            p += 1;
        }
        while p < c.len() && !is_space(c[p]) {
            p += 1;
        }
        p
    }

    /// The caret `by` rows away, at the place along the row the run began
    /// at; past the first row the start, past the last the end.
    fn row(&mut self, c: &[&str], p: usize, by: isize) -> usize {
        let lines = self.lines(c);
        let r = lines
            .iter()
            .position(|&(s, e)| s <= p && p <= e)
            .unwrap_or(0);
        let goal = *self.goal.get_or_insert(p - lines[r].0);
        let t = r as isize + by;
        if t < 0 {
            return 0;
        }
        let Some(&(s, e)) = lines.get(t as usize) else {
            return c.len();
        };
        s + goal.min(e - s)
    }

    fn delete(&mut self, c: &[&str], p: usize, a: usize, b: usize) -> bool {
        self.caret = p;
        if a >= b {
            return false;
        }
        self.value = c[..a].concat() + &c[b..].concat();
        self.caret = a;
        true
    }
}

/// The characters of `s`: its grapheme clusters (UAX #29), CR LF one.
fn chars(s: &str) -> Vec<&str> {
    s.graphemes(true).collect()
}

/// A space: a character whose first code point has White_Space. A line
/// break is one.
fn is_space(g: &str) -> bool {
    g.chars().next().is_some_and(char::is_whitespace)
}

fn is_break(g: &str) -> bool {
    matches!(g, "\n" | "\r\n" | "\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_field_has_its_caret_at_the_end() {
        let f = Field::new("héllo");
        assert_eq!((f.caret, f.selection()), (5, (5, 5)));
        assert_eq!(Field::default().rows, 1);
    }

    #[test]
    fn row_moves_keep_the_place_along_the_row() {
        let mut f = Field::new("abcdef\nab\nabcdef");
        f.multiline = true;
        f.select(5, 5);
        f.do_action("line-next");
        assert_eq!(f.caret, 9);
        f.do_action("line-next");
        assert_eq!(f.caret, 15);
        f.do_action("char-backward");
        f.do_action("line-previous");
        assert_eq!(f.caret, 9);
    }
}
