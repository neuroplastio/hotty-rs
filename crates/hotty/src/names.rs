//! Surface names (SDK.md §3.5).

/// Whether `s` is a surface name: 1 to 64 of `A`–`Z`, `a`–`z`, `0`–`9`, `_` and `-`.
pub fn valid_name(s: &str) -> bool {
    let _ = s;
    todo!("names: valid_name")
}

/// `s` made a surface name: each other character becomes `_`, it is cut to
/// 64, and an empty name becomes `_`.
pub fn surface_name(s: &str) -> String {
    let _ = s;
    todo!("names: surface_name")
}
