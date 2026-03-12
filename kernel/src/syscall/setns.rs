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
    fs::{
        file::{FileLike, InodeHandle, file_table::FileDesc},
        pseudofs::{NsCommonOps, NsFile},
        vfs::path::MountNamespace,
    },
    net::uts_ns::UtsNamespace,
    prelude::*,
    process::{
        CloneFlags, ContextSetNsAdminApi, NsProxy, NsProxyBuilder, PidFile, PidNamespace,
        PidNsForChildren, PidNsState, check_unsupported_ns_flags,
        credentials::capabilities::CapSet, posix_thread::AsPosixThread,
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

    let (new_ns_proxy, pid_ns_target) = if let Some(pid_file) = file.downcast_ref::<PidFile>() {
        build_proxy_from_pid_file(pid_file, ns_type_flags, ctx)?
    } else {
        build_proxy_from_ns_file(file.as_ref(), ns_type_flags, ctx)?
    };

    if let Some(new_ns_proxy) = new_ns_proxy {
        ctx.set_ns_proxy(Arc::new(new_ns_proxy));
    }

    if let Some(target_pid_ns) = pid_ns_target {
        set_pid_ns_for_children(target_pid_ns, ctx)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn build_proxy_from_pid_file(
    pid_file: &PidFile,
    flags: CloneFlags,
    ctx: &Context,
) -> Result<(Option<NsProxy>, Option<Arc<PidNamespace>>)> {
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

    let mut remaining_flags = flags;
    let mut pid_ns_target = None;

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

    if remaining_flags.contains(CloneFlags::CLONE_NEWPID) {
        pid_ns_target = Some(
            target_thread
                .as_posix_thread()
                .unwrap()
                .process()
                .active_pid_ns()
                .clone(),
        );
        remaining_flags.remove(CloneFlags::CLONE_NEWPID);
    }

    check_unsupported_ns_flags(remaining_flags)?;

    if flags.contains(CloneFlags::CLONE_NEWUTS) {
        let target_ns = target_proxy.uts_ns();
        set_uts_ns(&mut builder, target_ns, ctx)?;
    }

    if flags.contains(CloneFlags::CLONE_NEWNS) {
        let target_ns = target_proxy.mnt_ns();
        set_mnt_ns(&mut builder, target_ns, ctx)?;
    }

    // TODO: Support setting other namespaces from the target process.

    let proxy = if remaining_flags.is_empty() {
        None
    } else {
        Some(builder.build())
    };

    Ok((proxy, pid_ns_target))
}

fn build_proxy_from_ns_file(
    file: &dyn FileLike,
    flags: CloneFlags,
    ctx: &Context,
) -> Result<(Option<NsProxy>, Option<Arc<PidNamespace>>)> {
    if flags.contains(CloneFlags::CLONE_NEWUSER) {
        return_errno_with_message!(Errno::EINVAL, "setting a user namespace is not supported");
    }

    let mut remaining_flags = flags;
    let mut pid_ns_target = None;

    let inode_handle = file
        .downcast_ref::<InodeHandle>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a ns file"))?;

    let current_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_proxy = current_proxy.unwrap();

    let mut builder = NsProxyBuilder::new(current_proxy);

    if let Some(ns_file) = inode_handle.downcast_file_io::<NsFile<PidNamespace>>()? {
        if !remaining_flags.is_empty() && remaining_flags != CloneFlags::CLONE_NEWPID {
            return_errno_with_message!(
                Errno::EINVAL,
                "the flags do not match the type of the ns file"
            );
        }
        pid_ns_target = Some(ns_file.ns().clone());
        remaining_flags = CloneFlags::empty();
    }

    check_unsupported_ns_flags(remaining_flags)?;

    #[expect(clippy::nonminimal_bool)]
    let applied = false
        || try_apply_ns_from_inode::<UtsNamespace>(inode_handle, remaining_flags, |ns| {
            set_uts_ns(&mut builder, &ns, ctx)
        })?
        || try_apply_ns_from_inode::<MountNamespace>(inode_handle, remaining_flags, |ns| {
            set_mnt_ns(&mut builder, &ns, ctx)
        })?;
    // TODO: Support setting other namespaces from the ns file.

    if !applied && pid_ns_target.is_none() {
        return_errno_with_message!(Errno::EINVAL, "invalid flags are specified with a ns file");
    }

    let proxy = if applied { Some(builder.build()) } else { None };
    Ok((proxy, pid_ns_target))
}

fn try_apply_ns_from_inode<T: NsCommonOps>(
    inode_handle: &InodeHandle,
    flags: CloneFlags,
    apply: impl FnOnce(Arc<T>) -> Result<()>,
) -> Result<bool> {
    let Some(ns_file) = inode_handle.downcast_file_io::<NsFile<T>>()? else {
        return Ok(false);
    };

    if !flags.is_empty() && flags != T::TYPE.into() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the flags do not match the type of the ns file"
        );
    }

    apply(ns_file.ns().clone())?;
    Ok(true)
}

fn set_uts_ns(
    builder: &mut NsProxyBuilder,
    target_ns: &Arc<UtsNamespace>,
    ctx: &Context,
) -> Result<()> {
    // Verify the thread has SYS_ADMIN capability in the target namespace's owner
    // and the current user namespace.
    target_ns
        .owner_user_ns()
        .unwrap()
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
        .owner_user_ns()
        .unwrap()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;
    ctx.thread_local
        .borrow_user_ns()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

    if ctx.thread_local.is_fs_shared() {
        return_errno_with_message!(
            Errno::EINVAL,
            "setting a mount namespace is not allowed with shared filesystem information"
        );
    }

    // TODO: Are the checks above sufficient?

    builder.mnt_ns(target_ns.clone());

    Ok(())
}

fn set_pid_ns_for_children(target_ns: Arc<PidNamespace>, ctx: &Context) -> Result<()> {
    if target_ns.state() != PidNsState::Alive {
        return_errno_with_message!(Errno::EINVAL, "the target pid namespace is not alive");
    }

    target_ns
        .owner_user_ns()
        .unwrap()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;
    ctx.thread_local
        .borrow_user_ns()
        .check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

    let current_active = ctx.process.active_pid_ns();
    if !current_active.is_same_or_ancestor_of(&target_ns) {
        return_errno_with_message!(
            Errno::EINVAL,
            "the target pid namespace must be the current one or a descendant"
        );
    }

    let mut pid_ns_for_children = ctx.posix_thread.pid_ns_for_children().lock();
    *pid_ns_for_children = if Arc::ptr_eq(current_active, &target_ns) {
        PidNsForChildren::SameAsActive
    } else {
        PidNsForChildren::Target(target_ns)
    };

    Ok(())
}
