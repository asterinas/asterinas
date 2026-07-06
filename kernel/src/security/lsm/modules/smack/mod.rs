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
        LsmBprmHook, LsmCapabilityHook, LsmFileHook, LsmInodeHook, LsmMmapHook, LsmPathHook,
        LsmSocketHook,
    },
};
use crate::{
    fs::{
        file::Permission,
        vfs::{inode::Inode, path::Path},
    },
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
        let _ = xattr::exec_label(context.executable().inode().as_ref())?;

        if current_thread_has_mac_override() {
            return Ok(());
        }

        check_path_access(context.executable(), SmackAccess::EXECUTE)
    }

    fn on_bprm_committed_creds(&self, context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        let task_state = context.credentials().smack_task_state();
        let exec_label = task_state
            .exec_label()
            .cloned()
            .or(xattr::exec_label(context.executable().inode().as_ref())?);
        let Some(exec_label) = exec_label else {
            return Ok(());
        };

        context
            .credentials()
            .set_smack_task_state(task_state.with_current_label(exec_label));
        Ok(())
    }
}

impl LsmInodeHook for SmackLsm {
    fn on_inode_permission(&self, context: &lsm_hooks::InodePermissionContext<'_>) -> Result<()> {
        check_permission(context.inode(), context.permission())
    }
}

impl LsmFileHook for SmackLsm {
    fn on_file_permission(&self, context: &lsm_hooks::FilePermissionContext<'_>) -> Result<()> {
        check_file_permission(context.path(), context.permission())
    }
}

impl LsmPathHook for SmackLsm {
    fn on_path_create(&self, context: &lsm_hooks::PathCreateContext<'_>) -> Result<()> {
        check_path_access(context.parent(), SmackAccess::WRITE)
    }

    fn on_path_post_create(&self, context: &lsm_hooks::PathPostCreateContext<'_>) -> Result<()> {
        set_created_inode_label(context.parent(), context.child())
    }

    fn on_path_link(&self, context: &lsm_hooks::PathLinkContext<'_>) -> Result<()> {
        check_path_access(context.old_path(), SmackAccess::READ)?;
        check_path_access(context.new_parent(), SmackAccess::WRITE)
    }

    fn on_path_unlink(&self, context: &lsm_hooks::PathUnlinkContext<'_>) -> Result<()> {
        check_path_access(context.parent(), SmackAccess::WRITE)?;
        check_inode_access(context.child(), SmackAccess::WRITE)
    }

    fn on_path_rename(&self, context: &lsm_hooks::PathRenameContext<'_>) -> Result<()> {
        check_path_access(context.old_parent(), SmackAccess::WRITE)?;
        check_inode_access(context.old_child(), SmackAccess::WRITE)?;
        check_path_access(context.new_parent(), SmackAccess::WRITE)?;
        if let Some(new_child) = context.new_child() {
            check_inode_access(new_child, SmackAccess::WRITE)?;
        }
        Ok(())
    }

    fn on_path_setattr(&self, context: &lsm_hooks::PathSetattrContext<'_>) -> Result<()> {
        check_path_access(context.path(), SmackAccess::WRITE)
    }
}

impl LsmMmapHook for SmackLsm {
    fn on_mmap_file(&self, context: &lsm_hooks::MmapFileContext<'_>) -> Result<()> {
        check_mmap(context.path(), context.perms())
    }
}

impl LsmSocketHook for SmackLsm {
    fn on_socket_create(&self, context: &lsm_hooks::SocketCreateContext<'_>) -> Result<()> {
        set_socket_label(context.path())
    }

    fn on_socket_message(&self, context: &lsm_hooks::SocketMessageContext<'_>) -> Result<()> {
        check_socket_send_recv(context.path(), context.permission())
    }
}

/// Returns the Smack task state for a POSIX thread.
pub fn task_state(posix_thread: &PosixThread) -> SmackTaskState {
    posix_thread.credentials().smack_task_state()
}

/// Sets the current POSIX thread's Smack label.
pub fn set_current_label(label: &str) -> Result<()> {
    let label = parse_required_label(label)?;
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

/// Sets the current POSIX thread's one-shot Smack exec label.
pub fn set_exec_label(label: &str) -> Result<()> {
    set_optional_task_label(label, SmackTaskState::with_exec_label)
}

/// Sets the current POSIX thread's filesystem creation label.
pub fn set_fscreate_label(label: &str) -> Result<()> {
    set_optional_task_label(label, SmackTaskState::with_fscreate_label)
}

/// Sets the current POSIX thread's socket creation label.
pub fn set_sockcreate_label(label: &str) -> Result<()> {
    set_optional_task_label(label, SmackTaskState::with_sockcreate_label)
}

/// Loads Smack access rules.
pub fn load_rules(policy: &str) -> Result<usize> {
    current_thread_may_admin()?;
    access::load_rules(policy)
}

/// Returns loaded Smack access rules.
pub fn rules_as_text() -> String {
    access::rules_as_text()
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

pub(super) fn check_inode_access(inode: &dyn Inode, requested: SmackAccess) -> Result<()> {
    if current_thread_has_mac_override() {
        return Ok(());
    }

    let Some(subject) = current_subject_label() else {
        return Ok(());
    };
    let object = xattr::access_label(inode)?;
    access::check(&subject, &object, requested)
}

pub(super) fn check_path_access(path: &Path, requested: SmackAccess) -> Result<()> {
    check_inode_access(path.inode().as_ref(), requested)
}

pub(super) fn check_permission(inode: &dyn Inode, permission: Permission) -> Result<()> {
    check_inode_access(inode, access_from_permission(permission))
}

pub(super) fn check_file_permission(path: &Path, permission: Permission) -> Result<()> {
    check_path_access(path, access_from_permission(permission))
}

pub(super) fn check_mmap(path: &Path, perms: crate::vm::perms::VmPerms) -> Result<()> {
    if current_thread_has_mac_override() {
        return Ok(());
    }
    if !perms.intersects(crate::vm::perms::VmPerms::READ | crate::vm::perms::VmPerms::EXEC) {
        return Ok(());
    }

    let requested = if perms.contains(crate::vm::perms::VmPerms::EXEC) {
        SmackAccess::EXECUTE
    } else {
        SmackAccess::READ
    };
    check_path_access(path, requested)?;

    let Some(mmap_label) = xattr::mmap_label(path.inode().as_ref())? else {
        return Ok(());
    };
    let Some(subject) = current_subject_label() else {
        return Ok(());
    };
    access::check(&subject, &mmap_label, requested)
}

pub(super) fn set_created_inode_label(parent: &Path, child: &Path) -> Result<()> {
    let Some(subject) = current_subject_label() else {
        return Ok(());
    };
    let label = select_created_inode_label(parent, &subject)?;
    xattr::set_access_label(child.inode().as_ref(), &label)
}

pub(super) fn set_socket_label(path: &Path) -> Result<()> {
    let Some(subject) = current_subject_label() else {
        return Ok(());
    };
    let label = current_socket_create_label().unwrap_or(subject);
    xattr::set_access_label(path.inode().as_ref(), &label)
}

pub(super) fn check_socket_send_recv(path: &Path, permission: Permission) -> Result<()> {
    if current_thread_has_mac_override() {
        return Ok(());
    }

    let Some(subject) = current_subject_label() else {
        return Ok(());
    };
    let object = xattr::access_label(path.inode().as_ref())?;
    access::check(&subject, &object, access_from_permission(permission))
}

fn select_created_inode_label(parent: &Path, subject: &SmackLabel) -> Result<SmackLabel> {
    let Some(current_thread) = Thread::current() else {
        return Ok(subject.clone());
    };
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return Ok(subject.clone());
    };

    let task_state = posix_thread.credentials().smack_task_state();
    if let Some(label) = task_state.fscreate_label() {
        return Ok(label.clone());
    }

    let parent_inode = parent.inode().as_ref();
    if xattr::is_transmuting_directory(parent_inode)? {
        let parent_label = xattr::access_label(parent_inode)?;
        if access::check(subject, &parent_label, SmackAccess::TRANSMUTE).is_ok() {
            return Ok(parent_label);
        }
    }

    Ok(subject.clone())
}

fn access_from_permission(permission: Permission) -> SmackAccess {
    let mut access = SmackAccess::empty();
    if permission.may_read() {
        access |= SmackAccess::READ;
    }
    if permission.may_exec() {
        access |= SmackAccess::EXECUTE;
    }
    if permission.contains(Permission::MAY_APPEND) {
        access |= SmackAccess::APPEND;
    } else if permission.may_write() {
        access |= SmackAccess::WRITE;
    }

    access
}

fn set_optional_task_label(
    label: &str,
    update_state_fn: fn(&SmackTaskState, Option<SmackLabel>) -> SmackTaskState,
) -> Result<()> {
    let label = parse_optional_label(label)?;
    current_thread_may_admin()?;

    let current_thread = current_thread!();
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return_errno_with_message!(Errno::ESRCH, "the current thread is not a POSIX thread");
    };
    let task_state = posix_thread.credentials().smack_task_state();
    posix_thread.set_smack_task_state(update_state_fn(&task_state, label));
    Ok(())
}

fn parse_required_label(label: &str) -> Result<SmackLabel> {
    SmackLabel::parse(label.trim())
}

fn parse_optional_label(label: &str) -> Result<Option<SmackLabel>> {
    let label = label.trim();
    if label == "-" {
        return Ok(None);
    }

    SmackLabel::parse(label).map(Some)
}

fn current_thread_may_admin() -> Result<()> {
    let current_thread = current_thread!();
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return_errno_with_message!(Errno::ESRCH, "the current thread is not a POSIX thread");
    };

    lsm_hooks::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::MAC_ADMIN,
    ))
}

fn current_thread_has_mac_override() -> bool {
    let Some(thread) = Thread::current() else {
        return false;
    };
    let Some(posix_thread) = thread.as_posix_thread() else {
        return false;
    };

    posix_thread
        .credentials()
        .effective_capset()
        .contains(CapSet::MAC_OVERRIDE)
}

fn current_subject_label() -> Option<SmackLabel> {
    Some(
        Thread::current()?
            .as_posix_thread()?
            .credentials()
            .smack_task_state()
            .current_label()
            .clone(),
    )
}

fn current_socket_create_label() -> Option<SmackLabel> {
    Thread::current()?
        .as_posix_thread()?
        .credentials()
        .smack_task_state()
        .sockcreate_label()
        .cloned()
}
