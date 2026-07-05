// SPDX-License-Identifier: MPL-2.0

use super::label::SmackLabel;

/// Smack state attached to task credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmackTaskState {
    current_label: SmackLabel,
    exec_label: Option<SmackLabel>,
    fscreate_label: Option<SmackLabel>,
    previous_label: Option<SmackLabel>,
    sockcreate_label: Option<SmackLabel>,
}

impl SmackTaskState {
    /// Creates a task state with the default Smack floor label.
    pub fn new_floor() -> Self {
        Self {
            current_label: SmackLabel::floor(),
            exec_label: None,
            fscreate_label: None,
            previous_label: None,
            sockcreate_label: None,
        }
    }

    /// Creates a copy with a new current label.
    pub fn with_current_label(&self, label: SmackLabel) -> Self {
        let mut state = self.clone();
        state.previous_label = Some(core::mem::replace(&mut state.current_label, label));
        state.exec_label = None;
        state
    }

    /// Returns the current Smack label.
    pub fn current_label(&self) -> &SmackLabel {
        &self.current_label
    }
}

impl Default for SmackTaskState {
    fn default() -> Self {
        Self::new_floor()
    }
}
