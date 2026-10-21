//! Key names and keymaps (SDK.md §3.10, SPEC §10.2, §10.4).

/// What [`Keymap::lookup`] returns for a key the field types.
pub const INSERT: &str = "insert";

/// The SDK's keymap (SDK.md §3.10), the same in every SDK.
pub const TERMINAL_KEYS: &str = "";

/// What `mod` stands for (SPEC §10.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mod {
    /// `ctrl`.
    #[default]
    Ctrl,
    /// `super`.
    Super,
}

/// How a host reads bindings (SDK.md §3.10).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Platform {
    /// What `mod` stands for.
    pub modifier: Mod,
}

/// A set of bindings, key to action, read on a Platform.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {}

/// Reads a key's name (SDK.md §3.10).
pub fn parse_key(name: &str) -> Option<String> {
    let _ = name;
    todo!("keys")
}

/// Reads a key's name with `mod` standing for the Platform's.
pub fn read_key(platform: &Platform, name: &str) -> Option<String> {
    let _ = (platform, name);
    todo!("keys")
}

/// Reads input from the terminal as key names.
pub fn decode_keys(input: &[u8]) -> Vec<Option<String>> {
    let _ = input;
    todo!("keys")
}

/// The text a key types in a field.
pub fn key_text(key: &str) -> Option<String> {
    let _ = key;
    todo!("keys")
}

/// The default edit shortcuts for a `mod`.
pub fn default_edit(modifier: Mod) -> Vec<(String, Vec<String>)> {
    let _ = modifier;
    todo!("keys")
}

/// The Platform of a host's capabilities.
pub fn platform_of(caps: &crate::Caps) -> Platform {
    let _ = caps;
    todo!("keys")
}

/// Reads a `data-keys` value.
pub fn parse_keymap(platform: &Platform, value: &str) -> Keymap {
    let _ = (platform, value);
    todo!("keys")
}

/// Builds a field's keymap.
pub fn resolve<'a>(
    platform: &Platform,
    multiline: bool,
    values: impl IntoIterator<Item = &'a str>,
) -> Keymap {
    let _ = (platform, multiline, values.into_iter());
    todo!("keys")
}
