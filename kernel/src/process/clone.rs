// SPDX-License-Identifier: MPL-2.0

use core::{num::NonZeroU64, sync::atomic::Ordering};

use ostd::{cpu::context::UserContext, sync::RwArc, task::Task, user::UserContextApi};

use super::{
    posix_thread::{AsPosixThread, PosixThreadBuilder, ThreadName},
    process_table,
    process_vm::ProcessVm,
    rlimit::ResourceLimits,
    signal::{constants::SIGCHLD, sig_disposition::SigDispositions, sig_num::SigNum},
    Credentials, Pid, Process,
};
use crate::{
    cpu::LinuxAbi,
    current_userspace,
    fs::{
        file_table::{FdFlags, FileTable},
        thread_info::ThreadFsInfo,
    },
    prelude::*,
    process::{pid_file::PidFile, posix_thread::allocate_posix_tid},
    sched::Nice,
    thread::{AsThread, Tid},
};

bitflags! {
    #[derive(Default)]
    pub struct CloneFlags: u32 {
        const CLONE_VM      = 0x00000100;       /* Set if VM shared between processes.  */
        const CLONE_FS      = 0x00000200;       /* Set if fs info shared between processes.  */
        const CLONE_FILES   = 0x00000400;       /* Set if open files shared between processes.  */
        const CLONE_SIGHAND = 0x00000800;       /* Set if signal handlers shared.  */
        const CLONE_PIDFD   = 0x00001000;       /* Set if a pidfd should be placed in parent.  */
        const CLONE_PTRACE  = 0x00002000;       /* Set if tracing continues on the child.  */
        const CLONE_VFORK   = 0x00004000;       /* Set if the parent wants the child to wake it up on mm_release.  */
        const CLONE_PARENT  = 0x00008000;       /* Set if we want to have the same parent as the cloner.  */
        const CLONE_THREAD  = 0x00010000;       /* Set to add to same thread group.  */
        const CLONE_NEWNS   = 0x00020000;       /* Set to create new namespace.  */
        const CLONE_SYSVSEM = 0x00040000;       /* Set to shared SVID SEM_UNDO semantics.  */
        const CLONE_SETTLS  = 0x00080000;       /* Set TLS info.  */
        const CLONE_PARENT_SETTID = 0x00100000; /* Store TID in userlevel buffer before MM copy.  */
        const CLONE_CHILD_CLEARTID = 0x00200000;/* Register exit futex and memory location to clear.  */
        const CLONE_DETACHED = 0x00400000;      /* Create clone detached.  */
        const CLONE_UNTRACED = 0x00800000;      /* Set if the tracing process can't force CLONE_PTRACE on this clone.  */
        const CLONE_CHILD_SETTID = 0x01000000;  /* Store TID in userlevel buffer in the child.  */
        const CLONE_NEWCGROUP   = 0x02000000;	/* New cgroup namespace.  */
        const CLONE_NEWUTS	= 0x04000000;	    /* New utsname group.  */
        const CLONE_NEWIPC	= 0x08000000;	    /* New ipcs.  */
        const CLONE_NEWUSER	= 0x10000000;	    /* New user namespace.  */
        const CLONE_NEWPID	= 0x20000000;	    /* New pid namespace.  */
        const CLONE_NEWNET	= 0x40000000;	    /* New network namespace.  */
        const CLONE_IO	= 0x80000000;	        /* Clone I/O context.  */
    }
}

/// An internal structure to homogenize the arguments for `clone` and
/// `clone3`.
///
/// From the clone(2) man page:
///
/// ```
/// The following table shows the equivalence between the arguments
/// of clone() and the fields in the clone_args argument supplied to
/// clone3():
///     clone()         clone3()        Notes
///                     cl_args field
///     flags & ~0xff   flags           For most flags; details
///                                     below
///     parent_tid      pidfd           See CLONE_PIDFD
///     child_tid       child_tid       See CLONE_CHILD_SETTID
///     parent_tid      parent_tid      See CLONE_PARENT_SETTID
///     flags & 0xff    exit_signal
///     stack           stack
///     ---             stack_size
///     tls             tls             See CLONE_SETTLS
///     ---             set_tid         See below for details
///     ---             set_tid_size
///     ---             cgroup          See CLONE_INTO_CGROUP
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct CloneArgs {
    pub flags: CloneFlags,
    pub pidfd: Option<Vaddr>,
    pub child_tid: Vaddr,
    pub parent_tid: Option<Vaddr>,
    pub exit_signal: Option<SigNum>,
    pub stack: u64,
    pub stack_size: Option<NonZeroU64>,
    pub tls: u64,
    pub _set_tid: Option<u64>,
    pub _set_tid_size: Option<u64>,
    pub _cgroup: Option<u64>,
}

impl CloneArgs {
    /// Prepares a new [`CloneArgs`] based on the arguments for clone(2).
    pub fn for_clone(
        raw_flags: u64,
        parent_tid: Vaddr,
        child_tid: Vaddr,
        tls: u64,
        stack: u64,
    ) -> Result<Self> {
        const FLAG_MASK: u64 = 0xff;
        let flags = CloneFlags::from(raw_flags & !FLAG_MASK);
        let exit_signal = raw_flags & FLAG_MASK;
        // Disambiguate the `parent_tid` parameter. The field is used
        // both for `CLONE_PIDFD` and `CLONE_PARENT_SETTID`, so at
        // most only one can be specified.
        let (pidfd, parent_tid) = match (
            flags.contains(CloneFlags::CLONE_PIDFD),
            flags.contains(CloneFlags::CLONE_PARENT_SETTID),
        ) {
            (false, false) => (None, None),
            (true, false) => (Some(parent_tid), None),
            (false, true) => (None, Some(parent_tid)),
            (true, true) => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "CLONE_PIDFD was specified with CLONE_PARENT_SETTID"
                );
            }
        };

        Ok(Self {
            flags,
            pidfd,
            child_tid,
            parent_tid,
            exit_signal: (exit_signal != 0).then(|| SigNum::from_u8(exit_signal as u8)),
            stack,
            tls,
            ..Default::default()
        })
    }

    pub fn for_fork() -> Self {
        Self {
            exit_signal: Some(SIGCHLD),
            ..Default::default()
        }
    }

    pub fn for_vfork() -> Self {
        Self {
            flags: CloneFlags::CLONE_VFORK | CloneFlags::CLONE_VM,
            exit_signal: Some(SIGCHLD),
            ..Default::default()
        }
    }
}

impl From<u64> for CloneFlags {
    fn from(flags: u64) -> Self {
        // We use the lower 32 bits
        let clone_flags = (flags & 0xffff_ffff) as u32;
        CloneFlags::from_bits_truncate(clone_flags)
    }
}

impl CloneFlags {
    fn check_unsupported_flags(&self) -> Result<()> {
        let supported_flags = CloneFlags::CLONE_VM
            | CloneFlags::CLONE_FS
            | CloneFlags::CLONE_FILES
            | CloneFlags::CLONE_SIGHAND
            | CloneFlags::CLONE_PIDFD
            | CloneFlags::CLONE_THREAD
            | CloneFlags::CLONE_SYSVSEM
            | CloneFlags::CLONE_SETTLS
            | CloneFlags::CLONE_PARENT_SETTID
            | CloneFlags::CLONE_CHILD_SETTID
            | CloneFlags::CLONE_CHILD_CLEARTID
            | CloneFlags::CLONE_VFORK;
        let unsupported_flags = *self - supported_flags;
        if !unsupported_flags.is_empty() {
            warn!("contains unsupported clone flags: {:?}", unsupported_flags);
        }
        Ok(())
    }
}

/// Clone a child thread or child process.
///
/// FIXME: currently, the child process or thread will be scheduled to run at once,
/// but this may not be the expected behavior.
pub fn clone_child(
    ctx: &Context,
    parent_context: &UserContext,
    clone_args: CloneArgs,
) -> Result<Tid> {
    clone_args.flags.check_unsupported_flags()?;
    if clone_args.flags.contains(CloneFlags::CLONE_THREAD) {
        let child_task = clone_child_task(ctx, parent_context, clone_args)?;
        let child_thread = child_task.as_thread().unwrap();
        child_thread.run();

        let child_tid = child_thread.as_posix_thread().unwrap().tid();
        Ok(child_tid)
    } else {
        let child_process = clone_child_process(ctx, parent_context, clone_args)?;
        if clone_args.flags.contains(CloneFlags::CLONE_VFORK) {
            child_process.status().set_vfork_child(true);
        }

        child_process.run();

        if child_process.status().is_vfork_child() {
            let cond = || (!child_process.status().is_vfork_child()).then_some(());
            let current = ctx.process;
            current.children_wait_queue().wait_until(cond);
        }

        let child_pid = child_process.pid();
        Ok(child_pid)
    }
}

fn clone_child_task(
    ctx: &Context,
    parent_context: &UserContext,
    clone_args: CloneArgs,
) -> Result<Arc<Task>> {
    let clone_flags = clone_args.flags;

    // This combination is not valid, according to the Linux man pages. See
    // <https://www.man7.org/linux/man-pages/man2/clone.2.html>.
    if !clone_flags.contains(CloneFlags::CLONE_VM | CloneFlags::CLONE_SIGHAND) {
        return_errno_with_message!(
            Errno::EINVAL,
            "`CLONE_THREAD` without `CLONE_VM` and `CLONE_SIGHAND` is not valid"
        );
    }

    if clone_flags.contains(CloneFlags::CLONE_PIDFD) {
        return_errno_with_message!(
            Errno::EINVAL,
            "`CLONE_THREAD` cannot be used together with `CLONE_PIDFD`"
        );
    }

    let Context {
        process,
        thread_local,
        posix_thread,
        ..
    } = ctx;

    // Clone system V semaphore
    clone_sysvsem(clone_flags)?;

    // Clone file table
    let child_file_table = clone_files(thread_local.borrow_file_table().unwrap(), clone_flags);

    // Clone fs
    let child_fs = clone_fs(&thread_local.borrow_fs(), clone_flags);

    // Clone FPU context
    let child_fpu_context = thread_local.fpu().clone_context();

    let child_user_ctx = Box::new(clone_user_ctx(
        parent_context,
        clone_args.stack,
        clone_args.stack_size,
        clone_args.tls,
        clone_flags,
    ));

    // Inherit sigmask from current thread
    let sig_mask = posix_thread.sig_mask().load(Ordering::Relaxed).into();

    // Inherit the thread name.
    let thread_name = posix_thread.thread_name().lock().as_ref().cloned();

    let child_tid = allocate_posix_tid();
    let child_task = {
        let credentials = {
            let credentials = ctx.posix_thread.credentials();
            Credentials::new_from(&credentials)
        };

        let mut thread_builder = PosixThreadBuilder::new(child_tid, child_user_ctx, credentials)
            .process(posix_thread.weak_process())
            .thread_name(thread_name)
            .sig_mask(sig_mask)
            .file_table(child_file_table)
            .fs(child_fs)
            .fpu_context(child_fpu_context);

        // Deal with SETTID/CLEARTID flags
        clone_parent_settid(child_tid, clone_args.parent_tid, clone_flags)?;
        thread_builder = clone_child_cleartid(thread_builder, clone_args.child_tid, clone_flags);
        thread_builder = clone_child_settid(thread_builder, clone_args.child_tid, clone_flags);

        thread_builder.build()
    };

    process
        .tasks()
        .lock()
        .insert(child_task.clone())
        .map_err(|_| Error::with_message(Errno::EINTR, "the process has exited"))?;

    Ok(child_task)
}

fn clone_child_process(
    ctx: &Context,
    parent_context: &UserContext,
    clone_args: CloneArgs,
) -> Result<Arc<Process>> {
    let Context {
        process,
        thread_local,
        posix_thread,
        ..
    } = ctx;

    let clone_flags = clone_args.flags;

    // Clone the virtual memory space
    let child_process_vm = {
        let parent_process_vm = process.vm();
        clone_vm(parent_process_vm, clone_flags)?
    };

    // Clone the user context
    let child_user_ctx = Box::new(clone_user_ctx(
        parent_context,
        clone_args.stack,
        clone_args.stack_size,
        clone_args.tls,
        clone_flags,
    ));

    // Clone the file table
    let child_file_table = clone_files(thread_local.borrow_file_table().unwrap(), clone_flags);

    // Clone the filesystem information
    let child_fs = clone_fs(&thread_local.borrow_fs(), clone_flags);

    // Clone signal dispositions
    let child_sig_dispositions = clone_sighand(process.sig_dispositions(), clone_flags);

    // Clone System V semaphore
    clone_sysvsem(clone_flags)?;

    // Clone FPU context
    let child_fpu_context = thread_local.fpu().clone_context();

    // Inherit the parent's signal mask
    let child_sig_mask = posix_thread.sig_mask().load(Ordering::Relaxed).into();

    // Inherit the parent's resource limits
    let child_resource_limits = process.resource_limits().clone();

    // Inherit the parent's nice value
    let child_nice = process.nice().load(Ordering::Relaxed);

    let child_tid = allocate_posix_tid();

    let child = {
        let child_elf_path = process.executable_path();
        let mut child_thread_builder = {
            let child_thread_name = ThreadName::new_from_executable_path(&child_elf_path)?;

            let credentials = {
                let credentials = ctx.posix_thread.credentials();
                Credentials::new_from(&credentials)
            };

            PosixThreadBuilder::new(child_tid, child_user_ctx, credentials)
                .thread_name(Some(child_thread_name))
                .sig_mask(child_sig_mask)
                .file_table(child_file_table)
                .fs(child_fs)
                .fpu_context(child_fpu_context)
        };

        // Deal with SETTID/CLEARTID flags
        clone_parent_settid(child_tid, clone_args.parent_tid, clone_flags)?;
        child_thread_builder =
            clone_child_cleartid(child_thread_builder, clone_args.child_tid, clone_flags);
        child_thread_builder =
            clone_child_settid(child_thread_builder, clone_args.child_tid, clone_flags);

        create_child_process(
            child_tid,
            posix_thread.weak_process(),
            &child_elf_path,
            child_process_vm,
            child_resource_limits,
            child_nice,
            child_sig_dispositions,
            child_thread_builder,
        )
    };

    clone_pidfd(ctx, &child, clone_flags, clone_args.pidfd)?;

    if let Some(sig) = clone_args.exit_signal {
        child.set_exit_signal(sig);
    };

    // Sets parent process and group for child process.
    set_parent_and_group(process, &child);

    // Updates `has_child_subreaper` for the child process after inserting
    // it to its parent's children to make sure the `has_child_subreaper`
    // state of the child process will be consistent with its parent.
    if process.has_child_subreaper.load(Ordering::Relaxed) {
        child.has_child_subreaper.store(true, Ordering::Relaxed);
    }

    Ok(child)
}

fn clone_child_cleartid(
    child_builder: PosixThreadBuilder,
    child_tidptr: Vaddr,
    clone_flags: CloneFlags,
) -> PosixThreadBuilder {
    if clone_flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) {
        child_builder.clear_child_tid(child_tidptr)
    } else {
        child_builder
    }
}

fn clone_child_settid(
    child_builder: PosixThreadBuilder,
    child_tidptr: Vaddr,
    clone_flags: CloneFlags,
) -> PosixThreadBuilder {
    if clone_flags.contains(CloneFlags::CLONE_CHILD_SETTID) {
        child_builder.set_child_tid(child_tidptr)
    } else {
        child_builder
    }
}

fn clone_parent_settid(
    child_tid: Tid,
    parent_tidptr: Option<Vaddr>,
    clone_flags: CloneFlags,
) -> Result<()> {
    if let Some(addr) =
        parent_tidptr.filter(|_| clone_flags.contains(CloneFlags::CLONE_PARENT_SETTID))
    {
        current_userspace!().write_val(addr, &child_tid)?;
    }
    Ok(())
}

/// Clone child process vm. If CLONE_VM is set, both threads share the same root vmar.
/// Otherwise, fork a new copy-on-write vmar.
fn clone_vm(parent_process_vm: &ProcessVm, clone_flags: CloneFlags) -> Result<ProcessVm> {
    if clone_flags.contains(CloneFlags::CLONE_VM) {
        Ok(parent_process_vm.clone())
    } else {
        ProcessVm::fork_from(parent_process_vm)
    }
}

fn clone_user_ctx(
    parent_context: &UserContext,
    new_sp: u64,
    stack_size: Option<NonZeroU64>,
    tls: u64,
    clone_flags: CloneFlags,
) -> UserContext {
    let mut child_context = parent_context.clone();
    // The return value of child thread is zero
    child_context.set_syscall_ret(0);

    if clone_flags.contains(CloneFlags::CLONE_VM) && !clone_flags.contains(CloneFlags::CLONE_VFORK)
    {
        // If parent and child shares the same address space and not in vfork situation,
        // a new stack must be specified.
        debug_assert!(new_sp != 0);
    }
    if new_sp != 0 {
        // If stack size is not 0, the `new_sp` points to the BOTTOMMOST byte of stack.
        if let Some(size) = stack_size {
            child_context.set_stack_pointer((new_sp + size.get()) as usize)
        }
        // If stack size is 0, the new_sp points to the TOPMOST byte of stack.
        else {
            child_context.set_stack_pointer(new_sp as usize);
        }
    }
    if clone_flags.contains(CloneFlags::CLONE_SETTLS) {
        child_context.set_tls_pointer(tls as usize);
    }

    child_context
}

fn clone_fs(parent_fs: &Arc<ThreadFsInfo>, clone_flags: CloneFlags) -> Arc<ThreadFsInfo> {
    if clone_flags.contains(CloneFlags::CLONE_FS) {
        parent_fs.clone()
    } else {
        Arc::new(parent_fs.as_ref().clone())
    }
}

fn clone_files(parent_file_table: &RwArc<FileTable>, clone_flags: CloneFlags) -> RwArc<FileTable> {
    // if CLONE_FILES is set, the child and parent shares the same file table
    // Otherwise, the child will deep copy a new file table.
    // FIXME: the clone may not be deep copy.
    if clone_flags.contains(CloneFlags::CLONE_FILES) {
        parent_file_table.clone()
    } else {
        RwArc::new(parent_file_table.read().clone())
    }
}

fn clone_sighand(
    parent_sig_dispositions: &Arc<Mutex<SigDispositions>>,
    clone_flags: CloneFlags,
) -> Arc<Mutex<SigDispositions>> {
    // similar to CLONE_FILES
    if clone_flags.contains(CloneFlags::CLONE_SIGHAND) {
        parent_sig_dispositions.clone()
    } else {
        Arc::new(Mutex::new(*parent_sig_dispositions.lock()))
    }
}

fn clone_sysvsem(clone_flags: CloneFlags) -> Result<()> {
    if clone_flags.contains(CloneFlags::CLONE_SYSVSEM) {
        warn!("CLONE_SYSVSEM is not supported now");
    }
    Ok(())
}

fn clone_pidfd(
    ctx: &Context,
    child: &Arc<Process>,
    clone_flags: CloneFlags,
    pidfd_addr: Option<Vaddr>,
) -> Result<()> {
    if !clone_flags.contains(CloneFlags::CLONE_PIDFD) {
        return Ok(());
    }

    let pidfd_addr = pidfd_addr.unwrap();

    let fd = {
        let pid_file = PidFile::new(child.clone(), false);
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        file_table_locked.insert(Arc::new(pid_file), FdFlags::CLOEXEC)
    };

    // Since `write_val` may sleep, we cannot hold the file table lock during its execution.
    // FIXME: Should we remove the file from the file table if the write operation fails?
    match ctx.user_space().write_val(pidfd_addr, &fd) {
        Ok(()) => Ok(()),
        Err(e) => {
            let file_table = ctx.thread_local.borrow_file_table();
            let mut file_table_locked = file_table.unwrap().write();
            file_table_locked.close_file(fd);
            Err(e)
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn create_child_process(
    pid: Pid,
    parent: Weak<Process>,
    executable_path: &str,
    process_vm: ProcessVm,
    resource_limits: ResourceLimits,
    nice: Nice,
    sig_dispositions: Arc<Mutex<SigDispositions>>,
    thread_builder: PosixThreadBuilder,
) -> Arc<Process> {
    let child_proc = Process::new(
        pid,
        parent,
        executable_path.to_string(),
        process_vm,
        resource_limits,
        nice,
        sig_dispositions,
    );

    let child_task = thread_builder.process(Arc::downgrade(&child_proc)).build();
    child_proc.tasks().lock().insert(child_task).unwrap();

    child_proc
}

fn set_parent_and_group(parent: &Process, child: &Arc<Process>) {
    // Lock order: children of process -> process table -> group of process
    // -> group inner -> session inner
    let mut children_mut = parent.children().lock();

    let mut process_table_mut = process_table::process_table_mut();

    let process_group_mut = parent.process_group.lock();

    let process_group = process_group_mut.upgrade().unwrap();
    let mut process_group_inner = process_group.lock();

    // Put the child process in the parent's process group
    process_group_inner.insert_process(child.clone());
    *child.process_group.lock() = Arc::downgrade(&process_group);

    // Put the child process in the parent's `children` field
    children_mut.insert(child.pid(), child.clone());

    // Put the child process in the global table
    process_table_mut.insert(child.pid(), child.clone());
}
