// SPDX-License-Identifier: MPL-2.0

//! Smack major LSM foundation.

mod access;
mod label;
mod state;
mod xattr;

pub use self::{
    access::SmackAccess, label::SmackLabel, state::SmackTaskState, xattr::is_smack_xattr,
};
use super::super::{
    LsmFlags, LsmModule,
    hooks::{
        BprmCheckContext, BprmCommittedCredsContext, CapableContext, LsmAlienAccessHook,
        LsmBprmHook, LsmCapabilityHook,
    },
};
use crate::{
    prelude::*,
    process::{
        UserNamespace,
        credentials::capabilities::CapSet,
        posix_thread::{AsPosixThread, PosixThread},
    },
    security::lsm::hooks as lsm_hooks,
    thread::Thread,
};

pub(super) static SMACK_LSM: SmackLsm = SmackLsm;

/// The Smack major LSM.
pub(super) struct SmackLsm;

impl LsmModule for SmackLsm {
    fn name(&self) -> &'static str {
        "smack"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::LEGACY_MAJOR | LsmFlags::EXCLUSIVE
    }
}

impl LsmAlienAccessHook for SmackLsm {}

impl LsmCapabilityHook for SmackLsm {}

impl LsmBprmHook for SmackLsm {
    fn on_bprm_check_security(&self, context: &BprmCheckContext<'_>) -> Result<()> {
        let Some(thread) = Thread::current() else {
            return Ok(());
        };
        let Some(posix_thread) = thread.as_posix_thread() else {
            return Ok(());
        };
        if posix_thread
            .credentials()
            .effective_capset()
            .contains(CapSet::MAC_OVERRIDE)
        {
            return Ok(());
        }

        let subject = posix_thread
            .credentials()
            .smack_task_state()
            .current_label()
            .clone();
        let object = xattr::access_label(context.executable().inode())?;
        access::check(&subject, &object, SmackAccess::EXECUTE)
    }

    fn on_bprm_committed_creds(&self, context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        let Some(exec_label) = xattr::exec_label(context.executable().inode())? else {
            return Ok(());
        };

        let task_state = context.credentials().smack_task_state();
        context
            .credentials()
            .set_smack_task_state(task_state.with_current_label(exec_label));
        Ok(())
    }
}

/// Returns the Smack task state for a POSIX thread.
pub fn task_state(posix_thread: &PosixThread) -> SmackTaskState {
    posix_thread.credentials().smack_task_state()
}

/// Sets the current POSIX thread's Smack label.
pub fn set_current_label(label: &str) -> Result<()> {
    let label = SmackLabel::parse(label.trim())?;
    let current_thread = current_thread!();
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return_errno_with_message!(Errno::ESRCH, "the current thread is not a POSIX thread");
    };

    lsm_hooks::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::MAC_ADMIN,
    ))?;

    let task_state = posix_thread.credentials().smack_task_state();
    posix_thread.set_smack_task_state(task_state.with_current_label(label));
    Ok(())
}

/// Checks whether a Smack xattr may be updated.
pub fn check_xattr_update(posix_thread: &PosixThread, name: &str, value: &[u8]) -> Result<()> {
    if !is_smack_xattr(name) {
        return Ok(());
    }
    let Some(xattr_name) = crate::fs::vfs::xattr::XattrName::try_from_full_name(name) else {
        return_errno_with_message!(Errno::EOPNOTSUPP, "invalid xattr namespace");
    };

    lsm_hooks::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::MAC_ADMIN,
    ))?;

    xattr::validate_update(xattr_name, value)
}

/// Checks whether a Smack xattr may be removed.
pub fn check_xattr_removal(posix_thread: &PosixThread, name: &str) -> Result<()> {
    if !is_smack_xattr(name) {
        return Ok(());
    }

    lsm_hooks::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::MAC_ADMIN,
    ))
}
