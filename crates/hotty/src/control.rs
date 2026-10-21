//! A command's control: its keys and values, in order (SDK.md §3.2).

use std::fmt;

/// A command's control (SPEC §3.2): an **ordered** list of keys and values.
/// Keys go out in the order they were added.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Control(Vec<(String, String)>);

impl Control {
    /// An empty control.
    pub fn new() -> Control {
        Control(Vec::new())
    }

    /// A key's value, and whether it is present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Whether the key is present.
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Sets a key's value: in place if it is present, at the end if not.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let (key, value) = (key.into(), value.into());
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(kv) => kv.1 = value,
            None => self.0.push((key, value)),
        }
    }

    /// The control with a key set, as [`Control::set`] sets it (SDK.md §3.2's `with`).
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Control {
        self.set(key, value);
        self
    }

    /// Removes a key, and returns its value if it was present.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        let i = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(i).1)
    }

    /// The keys and values, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// How many keys it has.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether it has no keys.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The action, `a`, or `""` when there is none.
    pub fn action(&self) -> &str {
        self.get("a").unwrap_or("")
    }

    pub(crate) fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.push((key.into(), value.into()));
    }
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// The control as the wire writes it: `key=value` joined by `:`, values
/// as they are (not cleaned).
impl fmt::Display for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (k, v)) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{k}={v}")?;
        }
        Ok(())
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Control {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Control {
        let mut c = Control::new();
        for (k, v) in iter {
            c.set(k, v);
        }
        c
    }
}

impl<K: Into<String>, V: Into<String>, const N: usize> From<[(K, V); N]> for Control {
    fn from(pairs: [(K, V); N]) -> Control {
        pairs.into_iter().collect()
    }
}

impl<'a> IntoIterator for &'a Control {
    type Item = (&'a str, &'a str);
    type IntoIter = Box<dyn ExactSizeIterator<Item = (&'a str, &'a str)> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}
