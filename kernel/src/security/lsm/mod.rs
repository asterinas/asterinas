// SPDX-License-Identifier: MPL-2.0

//! The Linux Security Module (LSM) framework.
//!
//! LSM lets the kernel route security-sensitive operations through a stack of
//! built-in policy modules. Each module can implement shared hook traits and
//! inspect common hook contexts before allowing or rejecting an operation.
//!
//! This module defines the common LSM traits and hook contexts shared by
//! built-in modules such as `capability`, `yama`, and `apparmor`. Module
//! selection follows the `lsm=` and legacy `security=` kernel command-line
//! parameters.

pub mod hooks;
mod modules;

pub mod yama {
    pub use super::modules::yama::{YamaScope, get_scope, set_scope};
}

pub mod apparmor {
    pub use super::modules::apparmor::{
        AppArmorMode, AppArmorPolicyOperation, AppArmorProfileName, AppArmorTaskState,
        has_binary_policy_magic, load_binary_policy, load_policy, profile_summaries,
        remove_profile_by_name, root_namespace_name, task_state_for_profile,
    };
}

use self::hooks::{LsmAlienAccessHook, LsmBprmHook, LsmCapabilityHook, LsmFileHook};
pub use self::{
    apparmor::{AppArmorMode, AppArmorPolicyOperation, AppArmorProfileName, AppArmorTaskState},
    hooks::{
        BprmCheckContext, BprmCommittedCredsContext, CapableContext, FileCreateContext,
        FileCreateKind, FileDeleteContext, FileDeleteKind, FileLinkContext, FileOpenContext,
        FileRenameContext, FileSetattrContext, FileSetattrKind,
    },
    yama::YamaScope,
};
use crate::{prelude::*, process::posix_thread::PosixThread};

bitflags! {
    /// LSM module flags.
    pub struct LsmFlags: u32 {
        /// Marks a module as selectable through the legacy `security=` parameter.
        const LEGACY_MAJOR = 1 << 0;
        /// Marks a module as mutually exclusive with other exclusive modules.
        const EXCLUSIVE = 1 << 1;
    }
}

/// The common interface for built-in LSM modules.
trait LsmModule: LsmAlienAccessHook + LsmBprmHook + LsmCapabilityHook + LsmFileHook + Sync {
    /// Returns the module name.
    fn name(&self) -> &'static str;

    /// Returns the module flags.
    fn flags(&self) -> LsmFlags;
}

/// Returns whether the Yama LSM is enabled.
pub fn is_yama_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "yama")
}

/// Returns whether the AppArmor LSM is enabled.
pub fn is_apparmor_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "apparmor")
}

/// Returns the AppArmor task state for a POSIX thread if the module is active.
pub fn apparmor_task_state(posix_thread: &PosixThread) -> Option<AppArmorTaskState> {
    is_apparmor_enabled().then(|| posix_thread.credentials().apparmor_task_state())
}

/// Loads, replaces, or removes an AppArmor profile from policy text.
pub fn load_apparmor_policy(policy_text: &str) -> Result<()> {
    apparmor::load_policy(policy_text)
}

/// Loads, replaces, or removes an AppArmor profile from binary policy data.
pub fn load_apparmor_binary_policy(
    policy: &[u8],
    expected_operation: AppArmorPolicyOperation,
) -> Result<()> {
    apparmor::load_binary_policy(policy, expected_operation)
}

/// Returns whether the data starts with the AppArmor binary policy magic.
pub fn has_apparmor_binary_policy_magic(policy: &[u8]) -> bool {
    apparmor::has_binary_policy_magic(policy)
}

/// Removes a loaded AppArmor profile by name.
pub fn remove_apparmor_profile_by_name(profile_name: &str) -> Result<()> {
    apparmor::remove_profile_by_name(profile_name)
}

/// Returns summaries of the implicit and loaded AppArmor profiles.
pub fn apparmor_profile_summaries() -> Vec<(AppArmorProfileName, AppArmorMode)> {
    apparmor::profile_summaries()
}

/// Returns the root AppArmor policy namespace name.
pub fn apparmor_root_namespace_name() -> &'static str {
    apparmor::root_namespace_name()
}

/// Creates task state for a loaded AppArmor profile name.
pub fn apparmor_task_state_for_profile(profile_name: &str) -> Result<AppArmorTaskState> {
    apparmor::task_state_for_profile(profile_name)
}

pub(super) fn init() {
    for module in modules::active_modules() {
        info!("[kernel] LSM module enabled: {}", module.name());
    }
}

/// Runs the LSM stack for a capability check.
pub fn capable(context: CapableContext<'_>) -> Result<()> {
    hooks::on_capable(context)
}

/// Runs the LSM stack for an executable image check.
pub fn bprm_check_security(context: &BprmCheckContext<'_>) -> Result<()> {
    hooks::on_bprm_check_security(context)
}

/// Runs the LSM stack after executable credentials are committed.
pub fn bprm_committed_creds(context: &BprmCommittedCredsContext<'_>) -> Result<()> {
    hooks::on_bprm_committed_creds(context)
}

/// Runs the LSM stack for a file open check.
pub fn file_open(context: &FileOpenContext<'_>) -> Result<()> {
    hooks::on_file_open(context)
}

/// Runs the LSM stack for a file creation check.
pub fn file_create(context: &FileCreateContext<'_>) -> Result<()> {
    hooks::on_file_create(context)
}

/// Runs the LSM stack for a file deletion check.
pub fn file_delete(context: &FileDeleteContext<'_>) -> Result<()> {
    hooks::on_file_delete(context)
}

/// Runs the LSM stack for a file link check.
pub fn file_link(context: &FileLinkContext<'_>) -> Result<()> {
    hooks::on_file_link(context)
}

/// Runs the LSM stack for a file rename check.
pub fn file_rename(context: &FileRenameContext<'_>) -> Result<()> {
    hooks::on_file_rename(context)
}

/// Runs the LSM stack for a file attribute-change check.
pub fn file_setattr(context: &FileSetattrContext<'_>) -> Result<()> {
    hooks::on_file_setattr(context)
}
