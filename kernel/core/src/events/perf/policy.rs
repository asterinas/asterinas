// SPDX-License-Identifier: MPL-2.0

//! The system-wide access policy for opening thread performance events.

use core::sync::atomic::{AtomicI32, Ordering};

use ostd::pmu::EventConfig;

use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

const USER_ONLY: i32 = 2;
const PRIVILEGED_ONLY: i32 = 3;

// This setting is independent of event lifetime and target credentials. An
// open observes a single policy value; already-open events retain their access.
static PERF_EVENT_PARANOID: AtomicI32 = AtomicI32::new(PRIVILEGED_ONLY);

/// Returns the current value of `kernel.perf_event_paranoid`.
pub(crate) fn paranoid() -> i32 {
    PERF_EVENT_PARANOID.load(Ordering::Relaxed)
}

/// Sets `kernel.perf_event_paranoid` on behalf of an administrator.
pub(crate) fn set_paranoid(level: i32) -> Result<()> {
    let caller = current_thread!();
    let caller = caller.as_posix_thread().ok_or_else(|| {
        Error::with_message(Errno::EPERM, "a userspace administrator is required")
    })?;
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        caller,
        CapSet::SYS_ADMIN,
    ))?;

    PERF_EVENT_PARANOID.store(level, Ordering::Relaxed);
    Ok(())
}

/// Checks the requested counting scope against the current policy.
pub(crate) fn check_open(config: &EventConfig, ctx: &Context) -> Result<()> {
    let level = paranoid();
    let requires_privilege =
        level >= PRIVILEGED_ONLY || (level >= USER_ONLY && config.is_kernel_counted());
    if !requires_privilege {
        return Ok(());
    }

    // The PMU exposes system-wide hardware resources.
    let init_user_ns = UserNamespace::get_init_singleton();
    let is_capable_fn = |cap| {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            init_user_ns.as_ref(),
            ctx.posix_thread,
            cap,
        ))
        .is_ok()
    };
    if is_capable_fn(CapSet::PERFMON) || is_capable_fn(CapSet::SYS_ADMIN) {
        return Ok(());
    }

    if level >= PRIVILEGED_ONLY {
        return_errno_with_message!(Errno::EPERM, "CAP_PERFMON or CAP_SYS_ADMIN is required");
    }
    return_errno_with_message!(
        Errno::EACCES,
        "kernel-mode counting requires CAP_PERFMON or CAP_SYS_ADMIN"
    );
}
