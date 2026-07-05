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
    fs::vfs::path::Path,
    prelude::*,
    process::{Credentials, posix_thread::PosixThread},
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
