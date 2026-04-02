// SPDX-License-Identifier: MPL-2.0

pub(crate) mod lsm;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
mod tsm;

pub(crate) use self::lsm::{
    BprmCheckContext, FileOpenContext, InodePermissionContext, PtraceAccessContext,
    PtraceAccessCreds, PtraceAccessKind, PtraceAccessMode,
};
use crate::{
    fs::vfs::{inode::Inode, xattr::XattrName},
    prelude::*,
};

pub(super) fn init() {
    lsm::init();

    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    tsm::init();
}

/// Runs the LSM stack for a ptrace-style access check.
pub(crate) fn ptrace_access_check(context: &PtraceAccessContext<'_>) -> Result<()> {
    lsm::ptrace_access_check(context)
}

/// Runs the LSM stack for an executable image check.
pub(crate) fn bprm_check_security(context: &BprmCheckContext<'_>) -> Result<()> {
    lsm::bprm_check_security(context)
}

/// Runs the LSM stack for an inode permission check.
pub(crate) fn inode_permission(context: &InodePermissionContext<'_>) -> Result<()> {
    lsm::inode_permission(context)
}

/// Runs the LSM stack for a file-open check.
pub(crate) fn file_open(context: &FileOpenContext<'_>) -> Result<()> {
    lsm::file_open(context)
}

/// Returns whether an xattr is managed by the built-in Aster inode-security module.
pub(crate) fn is_aster_inode_xattr(name: &XattrName<'_>) -> bool {
    lsm::is_aster_inode_xattr(name)
}

/// Validates a managed Aster inode-security xattr value.
pub(crate) fn validate_aster_inode_xattr(name: &XattrName<'_>, value: &[u8]) -> Result<()> {
    lsm::validate_aster_inode_xattr(name, value)
}

/// Synchronizes a managed Aster inode-security xattr into the inode security state cache.
pub(crate) fn sync_aster_inode_xattr(
    inode: &Arc<dyn Inode>,
    name: &XattrName<'_>,
    value: Option<&[u8]>,
) -> Result<()> {
    lsm::sync_aster_inode_xattr(inode, name, value)
}
