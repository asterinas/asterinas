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

    /// Creates a copy with a new exec label.
    pub fn with_exec_label(&self, label: Option<SmackLabel>) -> Self {
        let mut state = self.clone();
        state.exec_label = label;
        state
    }

    /// Creates a copy with a new filesystem creation label.
    pub fn with_fscreate_label(&self, label: Option<SmackLabel>) -> Self {
        let mut state = self.clone();
        state.fscreate_label = label;
        state
    }

    /// Creates a copy with a new socket creation label.
    pub fn with_sockcreate_label(&self, label: Option<SmackLabel>) -> Self {
        let mut state = self.clone();
        state.sockcreate_label = label;
        state
    }

    /// Returns the current Smack label.
    pub fn current_label(&self) -> &SmackLabel {
        &self.current_label
    }

    /// Returns the exec label.
    pub fn exec_label(&self) -> Option<&SmackLabel> {
        self.exec_label.as_ref()
    }

    /// Returns the filesystem creation label.
    pub fn fscreate_label(&self) -> Option<&SmackLabel> {
        self.fscreate_label.as_ref()
    }

    /// Returns the previous current label.
    pub fn previous_label(&self) -> Option<&SmackLabel> {
        self.previous_label.as_ref()
    }

    /// Returns the socket creation label.
    pub fn sockcreate_label(&self) -> Option<&SmackLabel> {
        self.sockcreate_label.as_ref()
    }
}

impl Default for SmackTaskState {
    fn default() -> Self {
        Self::new_floor()
    }
}
