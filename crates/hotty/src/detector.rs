//! Whether the terminal is a host (SDK.md §3.8): a state machine with the
//! time passed in, in milliseconds.

use crate::VERSION;
use crate::commands::query;
use crate::messages::{Caps, Reply};

/// How long detection waits for any answer at all, in milliseconds.
pub const DETECT_TIMEOUT: u64 = 1500;
/// How long a DA1 that arrives before any reply leaves for the reply, in
/// milliseconds: it may answer a question asked before the query.
pub const DETECT_AFTER_DA1: u64 = 150;
/// How long detection waits after the reply to the query for the DA1
/// behind it, in milliseconds.
pub const DETECT_AFTER_REPLY: u64 = 300;

/// What detection has found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum State {
    /// Not known yet.
    #[default]
    Detecting,
    /// A host this SDK can use: surfaces.
    Native,
    /// A terminal that is not a host, or not one of this version: cells.
    Text,
}

/// Decides whether the terminal is a hotty host this SDK can use (SPEC §4,
/// SDK.md §3.8).
///
/// A reply to the query makes it one when it is `a=ok` and its capabilities
/// decode and name [`VERSION`]; any other reply to the query, an error
/// (`EVERSION` or another) or capabilities that do not decode or name
/// another version, makes it [`State::Text`] at once. Every input first lets
/// the time pass: whatever was due at or before `now` happens before the
/// input is taken.
///
/// ```
/// use hotty::{Detector, State};
///
/// let mut d = Detector::new(1);
/// let query = d.start(0); // write it to the terminal
/// # assert!(query.ends_with("\x1b[c"));
/// // … hand it what the terminal sends: d.reply(&r, now), d.da1(now) …
/// d.tick(1500);
/// assert_eq!(d.state(), State::Text); // nothing answered in time
/// ```
#[derive(Clone, Debug)]
pub struct Detector {
    n: u64,
    late: bool,
    state: State,
    caps: Option<Caps>,
    decided: bool,
    done: bool,
    timeout: Option<u64>,
    grace: Option<u64>,
    after: Option<u64>,
}

impl Detector {
    /// A Detector whose query is numbered `n`.
    pub fn new(n: u64) -> Detector {
        Detector {
            n,
            late: false,
            state: State::Detecting,
            caps: None,
            decided: false,
            done: false,
            timeout: None,
            grace: None,
            after: None,
        }
    }

    /// Asks for a late answer (SPEC §4): the first reply that answers the
    /// query once the state is [`State::Text`], and makes the terminal a
    /// host, does, with its capabilities. Still decided and done.
    #[must_use]
    pub fn late(mut self, late: bool) -> Detector {
        self.late = late;
        self
    }

    /// The query's number.
    pub fn n(&self) -> u64 {
        self.n
    }

    /// Whether the query asks for a late answer.
    pub fn is_late(&self) -> bool {
        self.late
    }

    /// What it has found.
    pub fn state(&self) -> State {
        self.state
    }

    /// The host's capabilities, when native.
    pub fn caps(&self) -> Option<&Caps> {
        self.caps.as_ref()
    }

    /// Whether the state is known: when the reply to the query arrives, if
    /// one does. A program that must not wait, such as a framework's
    /// adapter, acts on it.
    pub fn decided(&self) -> bool {
        self.decided
    }

    /// Whether nothing more of the detection will arrive. A program that
    /// hands the terminal on waits for it.
    pub fn done(&self) -> bool {
        self.done
    }

    /// When to call [`tick`](Detector::tick) next; `None` before
    /// [`start`](Detector::start) and once done.
    pub fn deadline(&self) -> Option<u64> {
        let timeout = self.timeout?;
        if self.done {
            return None;
        }
        if self.state == State::Detecting {
            Some(self.grace.map_or(timeout, |g| g.min(timeout)))
        } else {
            self.after
        }
    }

    /// Starts detection, and returns the query to send, with its DA1 fence.
    pub fn start(&mut self, now: u64) -> String {
        self.timeout = Some(now + DETECT_TIMEOUT);
        let q = query(self.n);
        if self.late { q.late() } else { q }.to_string()
    }

    fn fire(&mut self, now: u64) {
        let Some(timeout) = self.timeout else {
            return;
        };
        if self.done {
            return;
        }
        if self.state == State::Detecting {
            if now >= timeout || self.grace.is_some_and(|g| now >= g) {
                self.state = State::Text;
                self.decided = true;
                self.done = true;
            }
        } else if self.after.is_some_and(|a| now >= a) {
            self.done = true;
        }
    }

    /// The time is now.
    pub fn tick(&mut self, now: u64) {
        self.fire(now);
    }

    /// A DA1 answer arrived. Returns whether it was detection's: every DA1
    /// from the start until done is, and the caller swallows it.
    pub fn da1(&mut self, now: u64) -> bool {
        self.fire(now);
        if self.done {
            return false;
        }
        if self.state == State::Detecting {
            self.grace.get_or_insert(now + DETECT_AFTER_DA1);
            return true;
        }
        self.done = true;
        true
    }

    /// A reply arrived. Returns whether it answers the query (`re=q`, the
    /// query's `n`, `a=ok` or `a=err`): it is detection's whenever it
    /// arrives, and the caller swallows it. The first decides: a host when
    /// it is `a=ok` with capabilities that decode and name [`VERSION`],
    /// text otherwise, and done when the DA1 behind it arrives or 300 ms
    /// later. A late one changes nothing, unless the query asked for it:
    /// then the first that makes the terminal a host does.
    pub fn reply(&mut self, r: &Reply, now: u64) -> bool {
        self.fire(now);
        if r.re != "q" || r.n != Some(self.n) {
            return false;
        }
        let caps = r
            .caps
            .as_ref()
            .filter(|c| r.ok && c.v.as_deref() == Some(VERSION));
        match self.state {
            State::Detecting => {
                self.state = if caps.is_some() {
                    State::Native
                } else {
                    State::Text
                };
                self.decided = true;
                self.caps = caps.cloned();
                let after = now + DETECT_AFTER_REPLY;
                self.after = Some(self.timeout.map_or(after, |t| after.min(t)));
            }
            State::Text if self.late && caps.is_some() => {
                self.state = State::Native;
                self.caps = caps.cloned();
            }
            _ => {}
        }
        true
    }

    /// The input ended, or the caller gave up: [`State::Text`] if nothing
    /// was decided.
    pub fn end(&mut self, now: u64) {
        self.fire(now);
        if !self.decided {
            self.state = State::Text;
            self.decided = true;
        }
        self.done = true;
    }
}
