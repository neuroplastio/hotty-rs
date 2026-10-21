//! One command as OSC sequences (SDK.md §3.3).

use crate::control::Control;

/// A value with each character it may not hold made `_` (SPEC §3.2).
pub fn clean_value(v: &str) -> String {
    let _ = v;
    todo!("encode: clean_value")
}

/// One command as one or more OSC sequences (SDK.md §3.3).
pub fn encode(control: &Control, payload: &[u8]) -> String {
    let _ = (control, payload);
    todo!("encode: encode")
}
