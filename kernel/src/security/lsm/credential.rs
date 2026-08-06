// SPDX-License-Identifier: MPL-2.0

//! Security state carried by process credentials.

use super::modules::apparmor::Label;
use crate::prelude::*;

/// LSM state associated with one set of process credentials.
#[derive(Debug)]
pub struct CredentialSecurity {
    apparmor_label: RwLock<Label>,
}

impl CredentialSecurity {
    /// Creates security state for unconfined credentials.
    pub const fn new() -> Self {
        Self {
            apparmor_label: RwLock::new(Label::Unconfined),
        }
    }

    pub(in crate::security::lsm) const fn apparmor_label(&self) -> &RwLock<Label> {
        &self.apparmor_label
    }
}

impl Clone for CredentialSecurity {
    fn clone(&self) -> Self {
        Self {
            apparmor_label: RwLock::new(self.apparmor_label.read().clone()),
        }
    }
}

impl Default for CredentialSecurity {
    fn default() -> Self {
        Self::new()
    }
}
