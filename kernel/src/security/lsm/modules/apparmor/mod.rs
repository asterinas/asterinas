// SPDX-License-Identifier: MPL-2.0

//! AppArmor-like major LSM.

mod attachment;
mod binary;
mod capability;
mod dfa;
mod label;
mod loader;
mod namespace;
mod path;
mod policy;
mod policy_update;
mod profile;
mod state;

pub use self::{
    binary::AppArmorPolicyOperation,
    profile::AppArmorProfileName,
    state::{AppArmorMode, AppArmorTaskState},
};
use self::{policy::AppArmorPolicy, policy_update::AppArmorPolicyUpdate};
use super::super::{
    BprmCheckContext, BprmCommittedCredsContext, CapableContext, FileCreateContext,
    FileDeleteContext, FileLinkContext, FileOpenContext, FileRenameContext, FileSetattrContext,
    LsmFlags, LsmModule,
    hooks::{LsmAlienAccessHook, LsmBprmHook, LsmCapabilityHook, LsmFileHook},
};
use crate::{prelude::*, process::posix_thread::AsPosixThread, thread::Thread};

pub(super) static APPARMOR_LSM: AppArmorLsm = AppArmorLsm;

static POLICY: AppArmorPolicy = AppArmorPolicy::new();

/// An AppArmor-like major LSM.
pub(super) struct AppArmorLsm;

impl LsmModule for AppArmorLsm {
    fn name(&self) -> &'static str {
        "apparmor"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::LEGACY_MAJOR | LsmFlags::EXCLUSIVE
    }
}

impl LsmAlienAccessHook for AppArmorLsm {}

impl LsmBprmHook for AppArmorLsm {
    fn on_bprm_check_security(&self, context: &BprmCheckContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_execute(&task_state, context.path_resolver(), context.executable())
    }

    fn on_bprm_committed_creds(&self, context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        let task_state = context.credentials().apparmor_task_state();
        let new_task_state = POLICY.committed_exec_state(
            &task_state,
            context.path_resolver(),
            context.executable(),
        )?;
        context
            .credentials()
            .set_apparmor_task_state(new_task_state);
        Ok(())
    }
}

impl LsmCapabilityHook for AppArmorLsm {
    fn on_capable(&self, context: &CapableContext<'_>) -> Result<()> {
        let task_state = context.posix_thread().credentials().apparmor_task_state();
        POLICY.check_capability(&task_state, context.required_cap())
    }
}

impl LsmFileHook for AppArmorLsm {
    fn on_file_create(&self, context: &FileCreateContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_file_create(
            &task_state,
            context.path_resolver(),
            context.parent(),
            context.name(),
            context.kind(),
            context.access_mode(),
            context.status_flags(),
        )
    }

    fn on_file_delete(&self, context: &FileDeleteContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_file_delete(
            &task_state,
            context.path_resolver(),
            context.parent(),
            context.name(),
            context.kind(),
        )
    }

    fn on_file_link(&self, context: &FileLinkContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_file_link(
            &task_state,
            context.path_resolver(),
            context.source(),
            context.target_parent(),
            context.target_name(),
        )
    }

    fn on_file_open(&self, context: &FileOpenContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_file_open(
            &task_state,
            context.path_resolver(),
            context.path(),
            context.access_mode(),
            context.status_flags(),
        )
    }

    fn on_file_rename(&self, context: &FileRenameContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_file_rename(
            &task_state,
            context.path_resolver(),
            context.source(),
            context.new_parent(),
            context.new_name(),
        )
    }

    fn on_file_setattr(&self, context: &FileSetattrContext<'_>) -> Result<()> {
        let Some(task_state) = current_task_state() else {
            return Ok(());
        };

        POLICY.check_file_setattr(
            &task_state,
            context.path_resolver(),
            context.path(),
            context.kind(),
        )
    }
}

/// Loads, replaces, or removes an AppArmor profile from policy text.
pub fn load_policy(policy_text: &str) -> Result<()> {
    apply_policy_update(loader::parse_policy_load(policy_text)?)
}

/// Loads, replaces, or removes an AppArmor profile from binary policy data.
pub fn load_binary_policy(
    policy: &[u8],
    expected_operation: AppArmorPolicyOperation,
) -> Result<()> {
    apply_policy_update(binary::unpack_binary_policy(policy, expected_operation)?)
}

/// Returns whether the data starts with the AppArmor binary policy magic.
pub fn has_binary_policy_magic(policy: &[u8]) -> bool {
    binary::has_binary_policy_magic(policy)
}

fn apply_policy_update(update: AppArmorPolicyUpdate) -> Result<()> {
    match update {
        AppArmorPolicyUpdate::Replace(profile) => {
            POLICY.replace_profile(profile);
            Ok(())
        }
        AppArmorPolicyUpdate::ReplaceMany(profiles) => {
            for profile in profiles {
                POLICY.replace_profile(profile);
            }
            Ok(())
        }
        AppArmorPolicyUpdate::Remove(profile_name) => {
            if POLICY.remove_profile(&profile_name).is_none() {
                return_errno_with_message!(Errno::ENOENT, "the AppArmor profile is not loaded");
            }
            Ok(())
        }
    }
}

/// Returns summaries of the implicit and loaded AppArmor profiles.
pub fn profile_summaries() -> Vec<(AppArmorProfileName, AppArmorMode)> {
    POLICY.profile_summaries()
}

/// Returns the root AppArmor policy namespace name.
pub fn root_namespace_name() -> &'static str {
    POLICY.root_namespace_name()
}

/// Creates task state for a loaded AppArmor profile name.
pub fn task_state_for_profile(profile_name: &str) -> Result<AppArmorTaskState> {
    let profile_name = AppArmorProfileName::new(profile_name.to_string())?;
    let Some(mode) = POLICY.profile_mode(&profile_name) else {
        return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
    };

    Ok(AppArmorTaskState::new(profile_name, mode))
}

/// Removes a loaded AppArmor profile by name.
pub fn remove_profile_by_name(profile_name: &str) -> Result<()> {
    let profile_name = AppArmorProfileName::new(profile_name.to_string())?;
    if profile_name.is_unconfined() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the implicit unconfined AppArmor profile cannot be removed"
        );
    }
    if POLICY.remove_profile(&profile_name).is_none() {
        return_errno_with_message!(Errno::ENOENT, "the AppArmor profile is not loaded");
    }

    Ok(())
}

fn current_task_state() -> Option<AppArmorTaskState> {
    Thread::current()?
        .as_posix_thread()
        .map(|thread| thread.credentials().apparmor_task_state())
}
