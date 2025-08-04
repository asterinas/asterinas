// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{CloneFlags, ContextUnshareAdminApi},
    syscall::SyscallReturn,
};

pub fn sys_unshare(unshare_flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let mut flags = CloneFlags::from_bits(unshare_flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid `unshare` flags"))?;
    debug!("unshare flags = {:?}", flags);

    apply_implied_flags(&mut flags);
    check_flags(flags, ctx)?;

    if flags.contains(CloneFlags::CLONE_FILES) {
        ctx.unshare_files();
    }

    if flags.contains(CloneFlags::CLONE_FS) {
        ctx.unshare_fs();
    }

    if flags.contains(CloneFlags::CLONE_SYSVSEM) {
        ctx.unshare_sysvsem();
    }

    let ns_flags = flags.intersection(CloneFlags::CLONE_NS_FLAGS);
    if !ns_flags.is_empty() {
        ctx.unshare_namespaces(ns_flags)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn apply_implied_flags(flags: &mut CloneFlags) {
    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        *flags |= CloneFlags::CLONE_THREAD | CloneFlags::CLONE_FS;
    }

    if flags.contains(CloneFlags::CLONE_SIGHAND) {
        *flags |= CloneFlags::CLONE_THREAD;
    }

    if flags.contains(CloneFlags::CLONE_NEWNS) {
        *flags |= CloneFlags::CLONE_FS;
    }
}

fn check_flags(flags: CloneFlags, ctx: &Context) -> Result<()> {
    const VALID_FLAGS: CloneFlags = CloneFlags::CLONE_NS_FLAGS
        .union(CloneFlags::CLONE_FILES)
        .union(CloneFlags::CLONE_FS)
        .union(CloneFlags::CLONE_SYSVSEM)
        .union(CloneFlags::CLONE_THREAD)
        .union(CloneFlags::CLONE_VM)
        .union(CloneFlags::CLONE_SIGHAND);

    let invalid_flags = flags - VALID_FLAGS;
    if !invalid_flags.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "unsupported `unshare` flags");
    }

    if flags.intersects(CloneFlags::CLONE_THREAD | CloneFlags::CLONE_VM | CloneFlags::CLONE_SIGHAND)
        && ctx.process.tasks().lock().as_slice().len() != 1
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "`CLONE_THREAD`, `CLONE_VM`, and `CLONE_SIGHAND` can be specified only if the process is single-threaded"
        );
    }

    if flags.contains(CloneFlags::CLONE_VM) && ctx.user_space().is_vmar_shared() {
        return_errno_with_message!(
            Errno::EINVAL,
            "`CLONE_VM` can only be used when the VMAR is not shared"
        );
    }

    Ok(())
}
