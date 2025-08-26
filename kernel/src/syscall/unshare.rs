// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwArc;

use crate::{namespace::CLONE_NS_FLAGS, prelude::*, process::CloneFlags, syscall::SyscallReturn};

pub fn sys_unshare(unshare_flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let mut flags = CloneFlags::from_bits(unshare_flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid `unshare` flags"))?;
    debug!("unshare flags = {:?}", flags);

    apply_implied_flags(&mut flags);
    check_flags(flags, ctx)?;

    unshare_files(flags, ctx);
    unshare_fs(flags, ctx);
    unshare_sysvsem(flags, ctx);
    unshare_namespaces(flags, ctx)?;

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
    const VALID_FLAGS: CloneFlags = CLONE_NS_FLAGS
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
        return_errno_with_message!(Errno::EINVAL, "`CLONE_THREAD`, `CLONE_VM`, and `CLONE_SIGHAND` can be specified only if the process is single-threaded");
    }

    if flags.contains(CloneFlags::CLONE_VM) {
        let vmar_ref = ctx.thread_local.root_vmar().borrow();
        // If the VMAR is not shared, its reference count should be 2:
        // one reference is held by `ThreadLocal` and the other by `ProcessVm` in `Process`.
        if vmar_ref.as_ref().unwrap().reference_count() != 2 {
            return_errno_with_message!(
                Errno::EINVAL,
                "CLONE_VM can only be used when the VMAR is not shared"
            );
        }
    }

    Ok(())
}

pub(super) fn unshare_files(flags: CloneFlags, ctx: &Context) {
    if !flags.contains(CloneFlags::CLONE_FILES) {
        return;
    }

    let mut pthread_file_table = ctx.posix_thread.file_table().lock();

    let mut thread_local_file_table_ref = ctx.thread_local.borrow_file_table_mut();
    let thread_local_file_table = thread_local_file_table_ref.unwrap();

    let new_file_table = RwArc::new(thread_local_file_table.read().clone());

    *pthread_file_table = Some(new_file_table.clone_ro());
    *thread_local_file_table = new_file_table;
}

fn unshare_fs(flags: CloneFlags, ctx: &Context) {
    if !flags.contains(CloneFlags::CLONE_FS) {
        return;
    }

    let mut fs_ref = ctx.thread_local.borrow_fs_mut();
    let new_fs = fs_ref.as_ref().clone();
    *fs_ref = Arc::new(new_fs);
}

fn unshare_sysvsem(flags: CloneFlags, _ctx: &Context) {
    if !flags.contains(CloneFlags::CLONE_SYSVSEM) {
        return;
    }

    warn!("unsharing System V semaphore is not supported");
}

fn unshare_namespaces(flags: CloneFlags, ctx: &Context) -> Result<()> {
    if !flags.intersects(CLONE_NS_FLAGS) {
        return Ok(());
    }

    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        return_errno_with_message!(
            Errno::EINVAL,
            "cloning a new user namespace is not supported"
        );
    }

    let user_ns_ref = ctx.thread_local.borrow_user_ns();

    let mut pthread_ns_context = ctx.posix_thread.ns_context().lock();

    let mut thread_local_ns_context = ctx.thread_local.borrow_ns_context_mut();
    let thread_local_ns_context = thread_local_ns_context.unwrap();

    let new_ns_context =
        thread_local_ns_context.new_child(&user_ns_ref, flags, ctx.posix_thread)?;

    *pthread_ns_context = Some(new_ns_context.clone());
    *thread_local_ns_context = new_ns_context;

    if flags.contains(CloneFlags::CLONE_NEWNS) {
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .write()
            .switch_to_mnt_ns(thread_local_ns_context.mnt_ns())
            .unwrap();
    }

    Ok(())
}
