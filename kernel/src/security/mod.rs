// SPDX-License-Identifier: MPL-2.0

pub(crate) mod lsm;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
mod tsm;

pub(crate) use self::lsm::{
    PtraceAccessContext, PtraceAccessCreds, PtraceAccessKind, PtraceAccessMode,
};
use crate::prelude::*;

pub(super) fn init() {
    lsm::init();

    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    tsm::init();
}

/// Runs the LSM stack for a ptrace-style access check.
pub(crate) fn ptrace_access_check(context: &PtraceAccessContext<'_>) -> Result<()> {
    lsm::ptrace_access_check(context)
}
