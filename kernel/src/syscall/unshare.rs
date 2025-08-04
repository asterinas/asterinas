// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwArc;

use crate::{
    namespace::{NsContext, CLONE_NS_FLAGS},
    prelude::*,
    process::CloneFlags,
    syscall::SyscallReturn,
};

pub fn sys_unshare(unshare_flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = {
        let mut flags = CloneFlags::from_bits(unshare_flags)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid unshare flags"))?;
        apply_implied_flags(&mut flags);
        flags
    };

    debug!("unshare flags = {:?}", flags);

    check_flags(flags, ctx)?;

    unshare_files(flags, ctx);
    unshare_fs(flags, ctx);
    unshare_sysvsem(flags, ctx);
    unshare_namespaces(flags, ctx)?;

    Ok(SyscallReturn::Return(0))
}

fn apply_implied_flags(flags: &mut CloneFlags) {
    // Specifying CLONE_NEWIPC automatically implies CLONE_SYSVSEM.
    if flags.contains(CloneFlags::CLONE_NEWIPC) {
        *flags |= CloneFlags::CLONE_SYSVSEM;
    }

    // Specifying CLONE_NEWNS automatically implies CLONE_FS.
    if flags.contains(CloneFlags::CLONE_NEWNS) {
        *flags |= CloneFlags::CLONE_FS;
    }

    // Specifying CLONE_NEWPID automatically implies CLONE_THREAD.
    if flags.contains(CloneFlags::CLONE_NEWPID) {
        *flags |= CloneFlags::CLONE_THREAD;
    }

    // Specifying CLONE_NEWUSER automatically implies both CLONE_THREAD and CLONE_FS.
    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        *flags |= CloneFlags::CLONE_THREAD | CloneFlags::CLONE_FS;
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
        debug!("invalid flags: {:?}", invalid_flags);
        return_errno_with_message!(Errno::EINVAL, "unsupported unshare flags");
    }

    if flags.intersects(CloneFlags::CLONE_THREAD | CloneFlags::CLONE_VM | CloneFlags::CLONE_SIGHAND)
    {
        let is_single_threaded = ctx.process.tasks().lock().as_slice().len() == 1;
        if !is_single_threaded {
            return_errno_with_message!(Errno::EINVAL, "CLONE_THREAD, CLONE_VM, CLONE_SIGHAND can only be specified for single threaded process");
        }
    }

    Ok(())
}

fn unshare_files(flags: CloneFlags, ctx: &Context) {
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

    let mut pthread_ns_context = ctx.posix_thread.ns_context().lock();

    let mut thread_local_ns_context = ctx.thread_local.borrow_ns_context_mut();
    let thread_local_ns_context = thread_local_ns_context.unwrap();

    let new_ns_context = NsContext::clone_from(thread_local_ns_context, flags, ctx.posix_thread)?;

    *pthread_ns_context = Some(new_ns_context.clone_ro());
    *thread_local_ns_context = new_ns_context;

    Ok(())
}
