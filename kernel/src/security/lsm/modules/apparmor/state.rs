// SPDX-License-Identifier: MPL-2.0

//! Per-task AppArmor security state.

use super::label::AppArmorLabel;

/// AppArmor state attached to a task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppArmorTaskState {
    label: AppArmorLabel,
}

impl AppArmorTaskState {
    /// Creates the default unconfined AppArmor task state.
    pub fn new_unconfined() -> Self {
        Self {
            label: AppArmorLabel::new_unconfined(),
        }
    }
}

#[cfg(ktest)]
mod test {
    use alloc::string::ToString;
    use core::num::NonZeroU64;

    use ostd::prelude::*;

    use super::*;
    use crate::{
        process::credentials::Credentials,
        security::lsm::modules::apparmor::{
            label::AppArmorProfileIdentity, profile::AppArmorProfileName,
        },
    };

    fn label(name: &str, identity: u64) -> AppArmorLabel {
        AppArmorLabel::new_single(
            AppArmorProfileName::new(name.to_string()).unwrap(),
            AppArmorProfileIdentity::new(NonZeroU64::new(identity).unwrap()),
        )
        .unwrap()
    }

    #[ktest]
    fn credentials_copy_inherits_independent_task_state() {
        let inherited_state = AppArmorTaskState {
            label: label("inherited", 7),
        };
        let parent: Credentials = Credentials::new_root();
        parent.set_apparmor_task_state(inherited_state.clone());

        let child: Credentials = Credentials::new_from(&parent);
        assert_eq!(child.apparmor_task_state(), inherited_state);

        parent.set_apparmor_task_state(AppArmorTaskState::new_unconfined());
        assert_eq!(child.apparmor_task_state(), inherited_state);
    }
}
