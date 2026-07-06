// SPDX-License-Identifier: MPL-2.0

pub mod lsm;

use aster_rights::ReadWriteOp;
use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))] {
        mod tsm;
        mod tsm_mr;
    }
}

pub use self::lsm::SmackTaskState;
use crate::{
    fs::{
        file::Permission,
        vfs::{inode::Inode, path::Path},
    },
    prelude::*,
    process::{Credentials, posix_thread::PosixThread},
    vm::perms::VmPerms,
};

pub(super) fn init() {
    lsm::init();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tsm::init();
        tsm_mr::init();
    });
}

/// Returns whether the Smack LSM is enabled.
pub fn is_smack_enabled() -> bool {
    lsm::is_smack_enabled()
}

/// Returns the Smack task state for a POSIX thread if the module is active.
pub fn smack_task_state(posix_thread: &PosixThread) -> Option<SmackTaskState> {
    is_smack_enabled().then(|| lsm::smack::task_state(posix_thread))
}

/// Sets the current POSIX thread's Smack label.
pub fn set_current_smack_label(label: &str) -> Result<()> {
    if !is_smack_enabled() {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
    }

    lsm::smack::set_current_label(label)
}

/// Sets the current POSIX thread's one-shot Smack exec label.
pub fn set_current_smack_exec_label(label: &str) -> Result<()> {
    if !is_smack_enabled() {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
    }

    lsm::smack::set_exec_label(label)
}

/// Sets the current POSIX thread's Smack filesystem creation label.
pub fn set_current_smack_fscreate_label(label: &str) -> Result<()> {
    if !is_smack_enabled() {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
    }

    lsm::smack::set_fscreate_label(label)
}

/// Sets the current POSIX thread's Smack socket creation label.
pub fn set_current_smack_sockcreate_label(label: &str) -> Result<()> {
    if !is_smack_enabled() {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
    }

    lsm::smack::set_sockcreate_label(label)
}

/// Loads Smack access rules.
pub fn load_smack_rules(policy: &str) -> Result<usize> {
    if !is_smack_enabled() {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
    }

    lsm::smack::load_rules(policy)
}

/// Returns loaded Smack access rules.
pub fn smack_rules_as_text() -> Result<String> {
    if !is_smack_enabled() {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
    }

    Ok(lsm::smack::rules_as_text())
}

/// Checks whether a Smack xattr update is permitted and valid.
pub fn check_smack_xattr_update(
    posix_thread: &PosixThread,
    name: &str,
    value: &[u8],
) -> Result<()> {
    if !is_smack_enabled() || !lsm::smack::is_smack_xattr(name) {
        return Ok(());
    }

    lsm::smack::check_xattr_update(posix_thread, name, value)
}

/// Checks whether a Smack xattr removal is permitted.
pub fn check_smack_xattr_removal(posix_thread: &PosixThread, name: &str) -> Result<()> {
    if !is_smack_enabled() || !lsm::smack::is_smack_xattr(name) {
        return Ok(());
    }

    lsm::smack::check_xattr_removal(posix_thread, name)
}

/// Runs the LSM stack for an executable image check.
pub fn bprm_check_security(path: &Path) -> Result<()> {
    lsm::bprm_check_security(&lsm::BprmCheckContext::new(path))
}

/// Updates security state after credentials are committed for a new executable.
pub fn bprm_committed_creds(path: &Path, credentials: &Credentials<ReadWriteOp>) -> Result<()> {
    lsm::bprm_committed_creds(&lsm::BprmCommittedCredsContext::new(path, credentials))
}

/// Checks whether an inode permission request is allowed.
pub fn inode_permission(inode: &dyn Inode, permission: Permission) -> Result<()> {
    lsm::hooks::on_inode_permission(&lsm::hooks::InodePermissionContext::new(inode, permission))
}

/// Checks whether an opened file operation is allowed.
pub fn file_permission(path: &Path, permission: Permission) -> Result<()> {
    lsm::hooks::on_file_permission(&lsm::hooks::FilePermissionContext::new(path, permission))
}

/// Checks whether a child may be created under a directory.
pub fn path_create(parent: &Path) -> Result<()> {
    lsm::hooks::on_path_create(&lsm::hooks::PathCreateContext::new(parent))
}

/// Updates security state after a child has been created.
pub fn path_post_create(parent: &Path, child: &Path) -> Result<()> {
    lsm::hooks::on_path_post_create(&lsm::hooks::PathPostCreateContext::new(parent, child))
}

/// Checks whether a hard link may be created.
pub fn path_link(old_path: &Path, new_parent: &Path) -> Result<()> {
    lsm::hooks::on_path_link(&lsm::hooks::PathLinkContext::new(old_path, new_parent))
}

/// Checks whether a child may be removed.
pub fn path_unlink(parent: &Path, child: &dyn Inode) -> Result<()> {
    lsm::hooks::on_path_unlink(&lsm::hooks::PathUnlinkContext::new(parent, child))
}

/// Checks whether a child may be renamed.
pub fn path_rename(
    old_parent: &Path,
    old_child: &dyn Inode,
    new_parent: &Path,
    new_child: Option<&dyn Inode>,
) -> Result<()> {
    lsm::hooks::on_path_rename(&lsm::hooks::PathRenameContext::new(
        old_parent, old_child, new_parent, new_child,
    ))
}

/// Checks whether file metadata may be updated.
pub fn path_setattr(path: &Path) -> Result<()> {
    lsm::hooks::on_path_setattr(&lsm::hooks::PathSetattrContext::new(path))
}

/// Checks whether a file-backed memory mapping is allowed.
pub fn mmap_file(path: &Path, perms: VmPerms) -> Result<()> {
    lsm::hooks::on_mmap_file(&lsm::hooks::MmapFileContext::new(path, perms))
}

/// Labels a newly created socket.
pub fn socket_create(path: &Path) -> Result<()> {
    lsm::hooks::on_socket_create(&lsm::hooks::SocketCreateContext::new(path))
}

/// Checks whether a socket message operation is allowed.
pub fn socket_message(path: &Path, permission: Permission) -> Result<()> {
    lsm::hooks::on_socket_message(&lsm::hooks::SocketMessageContext::new(path, permission))
}
