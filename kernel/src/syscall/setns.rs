// SPDX-License-Identifier: MPL-2.0

//! This module implements the `setns` syscall.
//!
//! This syscall reassociates the calling thread with a namespace specified by a
//! file descriptor. The `flags` argument determines which type of namespace can be
//! joined.
//!
//! The file descriptor `fd` can refer to:
//! 1. A namespace file from `/proc/[pid]/ns/`.
//! 2. A `PidFile` opened by `pidfd_open` or by opening `/proc/[pid]` directory.

use crate::{
    fs::file_table::FileDesc,
    namespace::{
        check_unsupported_ns_flags, NsContext, NsContextCloneBuilder, UserNamespace, CLONE_NS_FLAGS,
    },
    prelude::*,
    process::{
        credentials::capabilities::CapSet, posix_thread::AsPosixThread, CloneFlags, PidFile,
    },
    syscall::SyscallReturn,
};

pub fn sys_setns(fd: FileDesc, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let ns_type_flags = CloneFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid setns flags"))?;

    let file = {
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.unwrap().read();
        file_table_locked.get_file(fd)?.clone()
    };

    let new_ns_context = if let Some(pid_file) = file.downcast_ref::<PidFile>() {
        build_context_from_pid_file(pid_file, ns_type_flags, ctx)?
    }
    // TODO: Support setting namespaces from `/proc/[pid]/ns`.
    else {
        return_errno_with_message!(
            Errno::EINVAL,
            "fd does not refer to a supported namespace file type"
        );
    };

    // Install the newly created namespace context.
    Arc::new(new_ns_context).install(ctx);

    Ok(SyscallReturn::Return(0))
}

fn build_context_from_pid_file(
    pid_file: &PidFile,
    flags: CloneFlags,
    ctx: &Context,
) -> Result<NsContext> {
    if flags.is_empty() {
        return_errno_with_message!(
            Errno::EINVAL,
            "flags must be specified when using a pid file"
        );
    }

    // Check for any flags that are not namespace-related.
    if !(flags - CLONE_NS_FLAGS).is_empty() {
        return_errno_with_message!(Errno::EINVAL, "invalid flags specified for pid file mode");
    }

    check_unsupported_ns_flags(flags)?;

    let target_thread = pid_file.process().main_thread();
    let target_context = target_thread.as_posix_thread().unwrap().ns_context().lock();
    let Some(target_context) = target_context.as_ref() else {
        return_errno_with_message!(Errno::ESRCH, "target process has exited");
    };

    let current_context = ctx.thread_local.borrow_ns_context();
    let current_context = current_context.unwrap();

    let mut clone_builder = NsContextCloneBuilder::new(current_context);

    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        let target_ns = target_context.user();
        set_user_ns(&mut clone_builder, target_ns, current_context.user(), ctx)?;
    }

    // TODO: Support setting other namespaces from the target process.

    Ok(clone_builder.build())
}

fn set_user_ns(
    clone_builder: &mut NsContextCloneBuilder,
    target_ns: &Arc<UserNamespace>,
    current_ns: &Arc<UserNamespace>,
    ctx: &Context,
) -> Result<()> {
    // Prevent a thread from re-entering the same user namespace.
    if Arc::ptr_eq(target_ns, current_ns) {
        return_errno_with_message!(
            Errno::EINVAL,
            "a thread cannot re-enter the same user namespace"
        );
    }

    // Disallow joining a new user namespace in multithreaded processes.
    if !is_single_threaded(ctx) {
        return_errno_with_message!(
            Errno::EINVAL,
            "multithreaded processes cannot join a new user namespace"
        );
    }

    // Verify the thread has SYS_ADMIN capability in the target namespace.
    target_ns.check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

    // Prevent joining the new namespace if the thread's filesystem state is shared.
    let fs = ctx.thread_local.borrow_fs();
    // FIXME: This check is brittle and will break in the future.
    // It assumes `ThreadFsInfo` is only stored in `ThreadLocal`, but a planned change
    // will also store it in `PosixThread`, invalidating this logic.
    if Arc::strong_count(&fs) != 1 {
        return_errno_with_message!(
            Errno::EINVAL,
            "cannot join a new namespace when the thread shares filesystem state with other threads"
        )
    }

    // TODO: Are the checks above sufficient?

    clone_builder.set_user(target_ns.clone());

    Ok(())
}

fn is_single_threaded(ctx: &Context) -> bool {
    ctx.process.tasks().lock().as_slice().len() == 1
}
