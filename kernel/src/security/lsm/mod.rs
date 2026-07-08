// SPDX-License-Identifier: MPL-2.0

//! The Linux Security Module (LSM) framework.
//!
//! LSM lets the kernel route security-sensitive operations through a stack of
//! built-in policy modules. Each module can implement shared hook traits and
//! inspect common hook contexts before allowing or rejecting an operation.
//!
//! This module defines the common LSM traits and hook contexts shared by
//! built-in modules such as `capability`, `yama`, and `smack`. Module
//! selection follows the `lsm=` and legacy `security=` kernel command-line
//! parameters.

pub mod hooks;
mod modules;

pub mod yama {
    pub use super::modules::yama::{YamaScope, get_scope, set_scope};
}

pub mod smack {
    pub use super::modules::smack::{
        SmackMountLabels, SmackTaskState, access_query_result_as_text, ambient_label_as_text,
        apply_mount_labels, change_rule, check_xattr_removal, check_xattr_update, is_smack_xattr,
        load_rules, logging_mode_as_text, mount_labels_from_options, onlycap_labels_as_text,
        query_access, revoke_subject, rules_as_text, set_ambient_label, set_current_label,
        set_exec_label, set_fscreate_label, set_logging_mode, set_onlycap_labels,
        set_sockcreate_label, task_state,
    };
}

use self::hooks::{
    LsmAlienAccessHook, LsmBprmHook, LsmCapabilityHook, LsmFileHook, LsmInodeHook, LsmMmapHook,
    LsmPathHook, LsmSocketHook,
};
pub use self::{
    hooks::{BprmCheckContext, BprmCommittedCredsContext},
    smack::{SmackMountLabels, SmackTaskState},
};
use crate::prelude::*;

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
trait LsmModule:
    LsmAlienAccessHook
    + LsmBprmHook
    + LsmCapabilityHook
    + LsmInodeHook
    + LsmFileHook
    + LsmPathHook
    + LsmMmapHook
    + LsmSocketHook
    + Sync
{
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

/// Returns whether the Smack LSM is enabled.
pub fn is_smack_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "smack")
}

pub(super) fn init() {
    for module in modules::active_modules() {
        info!("[kernel] LSM module enabled: {}", module.name());
    }
}

/// Runs the LSM stack for an executable image check.
pub fn bprm_check_security(context: &BprmCheckContext<'_>) -> Result<()> {
    hooks::on_bprm_check_security(context)
}

/// Runs the LSM stack after executable credentials are committed.
pub fn bprm_committed_creds(context: &BprmCommittedCredsContext<'_>) -> Result<()> {
    hooks::on_bprm_committed_creds(context)
}
