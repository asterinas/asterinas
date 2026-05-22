// SPDX-License-Identifier: MPL-2.0

pub mod lsm;

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_arch = x86_64, feature = cvm_guest))] {
        mod tsm;
        mod tsm_mr;
    }
}

pub use self::lsm::CapabilityReason;
use crate::{
    fs::file::{InodeMode, Permission},
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

pub(super) fn init() {
    lsm::init();

    #[cfg(target_arch = x86_64)]
    ostd::if_tdx_enabled!({
        tsm::init();
        tsm_mr::init();
    });
}

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
