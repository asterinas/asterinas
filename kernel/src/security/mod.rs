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

pub use self::lsm::CapabilityReason;
use crate::{
    fs::{
        file::{AccessMode, InodeMode, Permission, StatusFlags},
        vfs::{inode::Inode, path::Path, xattr::XattrName},
    },
    prelude::*,
    process::{
        Credentials, UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread,
    },
};

/// Returns whether the Yama LSM is enabled.
pub fn is_yama_enabled() -> bool {
    lsm::is_yama_enabled()
}

/// Returns whether the `aster_mac` LSM is enabled.
pub fn is_aster_mac_enabled() -> bool {
    lsm::is_aster_mac_enabled()
}

pub(super) fn init() {
    lsm::init();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tsm::init();
        tsm_mr::init();
    });
}

/// Runs the LSM stack for a capability check.
pub fn capable(
    user_namespace: &UserNamespace,
    capability: CapSet,
    posix_thread: &PosixThread,
    reason: CapabilityReason,
) -> Result<()> {
    lsm::capable(&lsm::CapableContext::new(
        user_namespace,
        posix_thread,
        capability,
        reason,
    ))
}

/// Runs the LSM stack for an executable image check.
pub fn bprm_check_security(path: &Path) -> Result<()> {
    lsm::bprm_check_security(&lsm::BprmCheckContext::new(path))
}

/// Updates security state after credentials are committed for a new executable.
pub fn bprm_committed_creds(path: &Path, credentials: &Credentials<ReadWriteOp>) -> Result<()> {
    lsm::bprm_committed_creds(&lsm::BprmCommittedCredsContext::new(path, credentials))
}

/// Runs the LSM stack for a DAC override decision on an inode.
pub fn inode_dac_override(
    mode: InodeMode,
    permission: Permission,
    posix_thread: &PosixThread,
) -> Result<Permission> {
    lsm::inode_dac_override(&lsm::InodeDacOverrideContext::new(
        mode,
        permission,
        posix_thread,
    ))
}

/// Runs the LSM stack for a file open check.
pub fn file_open(path: &Path, access_mode: AccessMode, status_flags: StatusFlags) -> Result<()> {
    lsm::file_open(&lsm::FileOpenContext::new(path, access_mode, status_flags))
}

/// Returns whether an xattr is owned by the `aster_mac` LSM.
pub fn is_aster_mac_inode_xattr(name: &XattrName<'_>) -> bool {
    lsm::is_aster_mac_inode_xattr(name)
}

/// Validates an `aster_mac` inode xattr value before storing it.
pub fn validate_aster_mac_inode_xattr(name: &XattrName<'_>, value: &[u8]) -> Result<()> {
    lsm::validate_aster_mac_inode_xattr(name, value)
}

/// Synchronizes the cached `aster_mac` inode state after an xattr update.
pub fn sync_aster_mac_inode_xattr(
    inode: &Arc<dyn Inode>,
    name: &XattrName<'_>,
    value: Option<&[u8]>,
) -> Result<()> {
    lsm::sync_aster_mac_inode_xattr(inode, name, value)
}
