//! Surface names (SDK.md §3.5).

use crate::MAX_NAME;

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `s` is a surface name: 1 to 64 of `A`–`Z`, `a`–`z`, `0`–`9`, `_`
/// and `-`.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME && s.chars().all(name_char)
}

/// `s` made a surface name: each character a name may not hold becomes `_`,
/// one for each code point, it is cut to 64, and an empty name becomes `_`.
pub fn surface_name(s: &str) -> String {
    if valid_name(s) {
        return s.to_string();
    }
    let out: String = s
        .chars()
        .take(MAX_NAME)
        .map(|c| if name_char(c) { c } else { '_' })
        .collect();
    if out.is_empty() { "_".to_string() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("a-b_C9"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name(&"x".repeat(65)));
        assert!(valid_name(&"x".repeat(64)));
        assert_eq!(surface_name("café:1"), "caf__1");
        assert_eq!(surface_name(""), "_");
        assert_eq!(surface_name(&"é".repeat(70)), "_".repeat(64));
    }
}
