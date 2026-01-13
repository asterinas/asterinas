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
    fs::{file_table::FileDesc, path::MountNamespace},
    net::uts_ns::UtsNamespace,
    prelude::*,
    process::{
        CloneFlags, ContextSetNsAdminApi, NsProxy, NsProxyBuilder, PidFile,
        check_unsupported_ns_flags, credentials::capabilities::CapSet, posix_thread::AsPosixThread,
    },
    syscall::SyscallReturn,
};

pub fn sys_setns(fd: FileDesc, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let ns_type_flags = CloneFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid `setns` flags"))?;
    debug!("setns flags = {:?}", ns_type_flags);

    let file = {
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.unwrap().read();
        file_table_locked.get_file(fd)?.clone()
    };

    let new_ns_proxy = if let Some(pid_file) = file.downcast_ref::<PidFile>() {
        build_proxy_from_pid_file(pid_file, ns_type_flags, ctx)?
    }
    // TODO: Support setting namespaces from `/proc/[pid]/ns`.
    else {
        return_errno_with_message!(
            Errno::EINVAL,
            "the FD does not refer to a supported namespace file"
        );
    };

    // Install the newly created `NsProxy`.
    ctx.set_ns_proxy(Arc::new(new_ns_proxy));

    Ok(SyscallReturn::Return(0))
}

fn build_proxy_from_pid_file(
    pid_file: &PidFile,
    flags: CloneFlags,
    ctx: &Context,
) -> Result<NsProxy> {
    if flags.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "flags must be specified with a PID file");
    }

    // Check for any flags that are not namespace-related.
    if !(flags - CloneFlags::CLONE_NS_FLAGS).is_empty() {
        return_errno_with_message!(Errno::EINVAL, "invalid flags are specified with a PID file");
    }

    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        return_errno_with_message!(Errno::EINVAL, "setting a user namespace is not supported");
    }

    check_unsupported_ns_flags(flags)?;

    let target_thread = pid_file
        .process_opt()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target process has been reaped"))?
        .main_thread();
    let target_proxy = target_thread.as_posix_thread().unwrap().ns_proxy().lock();
    let Some(target_proxy) = target_proxy.as_ref() else {
        return_errno_with_message!(Errno::ESRCH, "the target process has exited");
    };

    let current_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_proxy = current_proxy.unwrap();

    let mut builder = NsProxyBuilder::new(current_proxy);

    if flags.contains(CloneFlags::CLONE_NEWUTS) {
        let target_ns = target_proxy.uts_ns();
        set_uts_ns(&mut builder, target_ns, ctx)?;
    }

    if flags.contains(CloneFlags::CLONE_NEWNS) {
        if ctx.thread_local.is_fs_shared() {
            return_errno_with_message!(
                Errno::EINVAL,
                "setting a mount namespace is not allowed with shared filesystem information"
            );
        }

        let target_ns = target_proxy.mnt_ns();
        set_mnt_ns(&mut builder, target_ns, ctx)?;
    }

    // TODO: Support setting other namespaces from the target process.

    Ok(builder.build())
}

fn set_uts_ns(
    builder: &mut NsProxyBuilder,
    target_ns: &Arc<UtsNamespace>,
    ctx: &Context,
) -> Result<()> {
    // Verify the thread has SYS_ADMIN capability in the target namespace's owner
    // and the current user namespace.
    target_ns
        .owner_ns()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;
    ctx.thread_local
        .borrow_user_ns()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

    // TODO: Are the checks above sufficient?

    builder.uts_ns(target_ns.clone());

    Ok(())
}

fn set_mnt_ns(
    builder: &mut NsProxyBuilder,
    target_ns: &Arc<MountNamespace>,
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

    builder.mnt_ns(target_ns.clone());

    Ok(())
}
