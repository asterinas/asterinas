// SPDX-License-Identifier: MPL-2.0

//! This module implements the `setns` syscall.
//!
//! This syscall reassociates the calling thread with a namespace specified by a
//! file descriptor. The `flags` argument determines which type of namespace can be
//! joined.
//!
//! The file descriptor `fd` can refer to:
//! 1. A namespace file from `/proc/[pid]/ns/`.
//! 2. A `PidFile` opened by `pidfd_open` or open `/proc/[pid]` directory.

use ostd::sync::RwArc;

use crate::{
    fs::{file_handle::FileLike, file_table::FileDesc},
    namespace::{
        check_unsupported_ns_flags, NsContext, NsContextCloneBuilder, NsFile, NsType,
        UserNamespace, CLONE_NS_FLAGS,
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

    let new_ns_context = if let Some(ns_file) = as_ns_file(file.as_ref()) {
        build_context_from_ns_file(ns_file, ns_type_flags, ctx)?
    } else if let Some(pid_file) = file.downcast_ref::<PidFile>() {
        build_context_from_pid_file(pid_file, ns_type_flags, ctx)?
    } else {
        return_errno_with_message!(
            Errno::EINVAL,
            "fd does not refer to a supported namespace file type"
        );
    };

    // Install the newly created namespace context.
    NsContext::install(new_ns_context, ctx);

    Ok(SyscallReturn::Return(0))
}

fn as_ns_file(file: &dyn FileLike) -> Option<&NsFile> {
    file.inode()?.downcast_ref::<NsFile>()
}

fn build_context_from_ns_file(
    ns_file: &NsFile,
    flags: CloneFlags,
    ctx: &Context,
) -> Result<RwArc<NsContext>> {
    let target_ns = ns_file.ns();
    let target_ns_type_flag = CloneFlags::from(target_ns.type_());

    // The flags mush be specified and they must match the type of the target namespace file.
    if flags.is_empty() || flags != target_ns_type_flag {
        return_errno_with_message!(
            Errno::EINVAL,
            "setns flags do not match the type of the namespace file"
        );
    }

    check_unsupported_ns_flags(target_ns_type_flag)?;

    let current_context = ctx.thread_local.borrow_ns_context();
    let current_context_locked = current_context.unwrap().read();
    let mut clone_builder = NsContextCloneBuilder::new(&current_context_locked);

    // Based on the namespace type, add it to the builder.
    match target_ns.type_() {
        NsType::User => {
            let target_ns = Arc::downcast(target_ns.clone()).unwrap();
            set_user_ns(
                &mut clone_builder,
                target_ns,
                current_context_locked.user(),
                ctx,
            )?;
        }
        // TODO: Support other namespace types.
        _ => return_errno_with_message!(Errno::EINVAL, "the namespace to set is unsupported"),
    }

    Ok(clone_builder.build())
}

fn build_context_from_pid_file(
    pid_file: &PidFile,
    flags: CloneFlags,
    ctx: &Context,
) -> Result<RwArc<NsContext>> {
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

    // Lock target context and the current context.
    // Since we acquire only read locks for both, the lock order doesn't matter.

    let target_thread = pid_file.process().main_thread();
    let target_context = target_thread.as_posix_thread().unwrap().ns_context().lock();
    let Some(target_context) = target_context.as_ref() else {
        return_errno_with_message!(Errno::ESRCH, "target process has exited");
    };
    let target_context_locked = target_context.read();

    let current_context = ctx.thread_local.borrow_ns_context();
    let current_context_locked = current_context.unwrap().read();

    let mut clone_builder = NsContextCloneBuilder::new(&current_context_locked);

    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        let target_ns = target_context_locked.user().clone();
        set_user_ns(
            &mut clone_builder,
            target_ns,
            current_context_locked.user(),
            ctx,
        )?;
    }

    // TODO: Support setting other namespaces from the target process.

    Ok(clone_builder.build())
}

fn set_user_ns(
    clone_builder: &mut NsContextCloneBuilder,
    target_ns: Arc<UserNamespace>,
    current_ns: &Arc<UserNamespace>,
    ctx: &Context,
) -> Result<()> {
    // Prevent a thread from re-entering the same user namespace
    if Arc::ptr_eq(&target_ns, current_ns) {
        return_errno_with_message!(
            Errno::EINVAL,
            "a thread cannot re-enter the same user namespace"
        );
    }

    // Disallow joining a new user namespace in multithreaded processes
    if !is_single_threaded(ctx) {
        return_errno_with_message!(
            Errno::EINVAL,
            "multithreaded processes cannot join a new user namespace"
        );
    }

    // Verify the thread has SYS_ADMIN capability in the target namespace
    target_ns.check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

    let fs = ctx.thread_local.borrow_fs();
    // Prevent joining the new namespace if the thread's filesystem state is shared
    if Arc::strong_count(&fs) != 1 {
        return_errno_with_message!(
            Errno::EINVAL,
            "cannot join a new namespace when the thread shares filesystem state with other threads"
        )
    }

    // TODO: Are the checks above sufficient?

    clone_builder.set_user(target_ns);

    Ok(())
}

fn is_single_threaded(ctx: &Context) -> bool {
    ctx.process.tasks().lock().as_slice().len() == 1
}
