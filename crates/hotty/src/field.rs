//! A text field edited in cells (SDK.md §4.6).

/// A text field's value, caret and selection (SDK.md §4.6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {}

impl Field {
    /// Does an action; whether the value changed.
    pub fn do_action(&mut self, action: &str) -> bool {
        let _ = action;
        todo!("field")
    }

    /// Types text at the caret, in place of the selection.
    pub fn type_text(&mut self, text: &str) -> bool {
        let _ = text;
        todo!("field")
    }
}
