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
        file::{InodeMode, Permission},
        vfs::path::Path,
    },
    prelude::*,
    process::{
        Credentials, UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread,
    },
};

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

/// Updates security state after credentials are committed for a new executable.
pub fn bprm_committed_creds(_path: &Path, _credentials: &Credentials<ReadWriteOp>) -> Result<()> {
    Ok(())
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
