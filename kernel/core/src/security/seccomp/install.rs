// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{
    abi::{SockFilter, SockFprog},
    cbpf::{MAX_INSNS, VerifiedProgram},
};
use crate::{
    prelude::*,
    process::credentials::capabilities::CapSet,
    security::lsm::hooks::{self, CapableContext},
};

/// Installs a new filter after preparing it without modifying the effective policy.
pub(super) fn install_filter(ctx: &Context, flags: u32, fprog_addr: Vaddr) -> Result<()> {
    if flags != 0 {
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp filter flags");
    }

    let user_space = ctx.user_space();
    let fprog: SockFprog = user_space.read_val(fprog_addr)?;
    let len = fprog.len as usize;
    if len == 0 || len > MAX_INSNS {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp program length");
    }
    check_install_permission(ctx)?;
    if fprog.filter == 0 {
        return_errno_with_message!(Errno::EINVAL, "null seccomp instruction array");
    }

    let mut raw = Vec::<SockFilter>::new();
    raw.try_reserve_exact(len)
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate seccomp program"))?;
    raw.resize(
        len,
        SockFilter {
            code: 0,
            jt: 0,
            jf: 0,
            k: 0,
        },
    );
    user_space.read_bytes(fprog.filter as Vaddr, raw.as_mut_bytes())?;
    let program = VerifiedProgram::new(raw)?;

    // Only the calling thread can install its policy (TSYNC is unsupported).
    // No user access or verifier work occurs while holding the state lock.
    let old_state = ctx.posix_thread.seccomp_snapshot();
    let new_state = old_state.append(program)?;
    ctx.posix_thread.set_seccomp(new_state);
    Ok(())
}

fn check_install_permission(ctx: &Context) -> Result<()> {
    if ctx.posix_thread.credentials().no_new_privs() {
        return Ok(());
    }

    let user_ns = ctx.thread_local.borrow_user_ns();
    hooks::on_capable(CapableContext::new(
        &user_ns,
        ctx.posix_thread,
        CapSet::SYS_ADMIN,
    ))
    .map_err(|_| Error::with_message(Errno::EACCES, "seccomp installation is not permitted"))
}
