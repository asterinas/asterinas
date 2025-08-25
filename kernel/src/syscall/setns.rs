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
        check_unsupported_ns_flags, NsContext, NsContextCloneBuilder, UtsNamespace, CLONE_NS_FLAGS,
    },
    prelude::*,
    process::{
        credentials::capabilities::CapSet, posix_thread::AsPosixThread, CloneFlags, PidFile,
    },
    syscall::SyscallReturn,
};

pub fn sys_setns(fd: FileDesc, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let ns_type_flags = CloneFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid `setns` flags"))?;
    debug!("flags = {:?}", ns_type_flags);

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
            "the FD does not refer to a supported namespace file"
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
        return_errno_with_message!(Errno::EINVAL, "flags must be specified with a PID file");
    }

    // Check for any flags that are not namespace-related.
    if !(flags - CLONE_NS_FLAGS).is_empty() {
        return_errno_with_message!(Errno::EINVAL, "invalid flags are specified with a PID file");
    }

    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        return_errno_with_message!(Errno::EINVAL, "setting a user namespace is not supported");
    }

    check_unsupported_ns_flags(flags)?;

    let target_thread = pid_file.process().main_thread();
    let target_context = target_thread.as_posix_thread().unwrap().ns_context().lock();
    let Some(target_context) = target_context.as_ref() else {
        return_errno_with_message!(Errno::ESRCH, "the target process has exited");
    };

    let current_context = ctx.thread_local.borrow_ns_context();
    let current_context = current_context.unwrap();

    let mut clone_builder = NsContextCloneBuilder::new(current_context);

    if flags.contains(CloneFlags::CLONE_NEWUTS) {
        let target_ns = target_context.uts();
        set_uts_ns(&mut clone_builder, target_ns, ctx)?;
    }

    // TODO: Support setting other namespaces from the target process.

    Ok(clone_builder.build())
}

fn set_uts_ns(
    clone_builder: &mut NsContextCloneBuilder,
    target_ns: &Arc<UtsNamespace>,
    ctx: &Context,
) -> Result<()> {
    // Verify the thread has SYS_ADMIN capability in the target namespace's owner
    // and the current user namespace.
    target_ns
        .owner()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;
    ctx.thread_local
        .borrow_user_ns()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

    // TODO: Are the checks above sufficient?

    clone_builder.new_uts(target_ns.clone());

    Ok(())
}
