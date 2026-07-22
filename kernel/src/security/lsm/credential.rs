// SPDX-License-Identifier: MPL-2.0

//! LSM state attached to process credentials.

use ostd::sync::RwLock;

use super::modules::apparmor::AppArmorTaskState;

/// Security-module state associated with one set of process credentials.
#[derive(Debug)]
pub struct LsmCredentialState {
    apparmor: RwLock<AppArmorTaskState>,
}

impl LsmCredentialState {
    /// Creates the initial security-module credential state.
    pub fn new() -> Self {
        Self {
            apparmor: RwLock::new(AppArmorTaskState::new_unconfined()),
        }
    }

    /// Returns a snapshot of the AppArmor task state.
    pub fn apparmor_task_state(&self) -> AppArmorTaskState {
        self.apparmor.read().clone()
    }

    /// Replaces the AppArmor task state.
    pub fn set_apparmor_task_state(&self, task_state: AppArmorTaskState) {
        *self.apparmor.write() = task_state;
    }
}

impl Clone for LsmCredentialState {
    fn clone(&self) -> Self {
        Self {
            apparmor: RwLock::new(self.apparmor.read().clone()),
        }
    }
}
