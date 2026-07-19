// SPDX-License-Identifier: MPL-2.0

use aster_rights::ReadWriteOp;
#[cfg(target_arch = "x86_64")]
use ostd::arch::cpu::context::{FsBase, GsBase};
use ostd::{
    arch::cpu::context::{FpuContext, GeneralRegs, UserContext},
    mm::VmIo,
    sync::Waiter,
    user::UserContextApi,
};

use super::process_vm::activate_vmar;
use crate::{
    fs::vfs::{inode::Inode, path::Path},
    prelude::*,
    process::{
        ContextUnshareAdminApi, Credentials, Gid, Process, Uid, pid_table,
        posix_thread::{
            AsPosixThread, ContextPthreadAdminApi, ThreadLocal, ThreadName, ptrace::PtraceEvent,
            sigkill_other_threads,
        },
        process_vm::{MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS, ProcessVm},
        program_loader::{ProgramToLoad, elf::ElfLoadInfo},
        signal::{
            HandlePendingSignal, PauseReason, SigStack,
            constants::{SIGCHLD, SIGKILL},
            signals::kernel::KernelSignal,
        },
    },
    vm::vmar::VmarHandle,
};

pub fn do_execve(
    elf_file: Path,
    thread_name: ThreadName,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<()> {
    // FIXME: A malicious user could cause a kernel panic by exhausting available memory.
    // Currently, the implementation reads up to `MAX_NR_STRING_ARGS` arguments, each up to
    // `MAX_LEN_STRING_ARG` in length, without first verifying the total combined size.
    // To prevent excessive memory allocation, a preliminary check should sum the lengths
    // of all strings to enforce a sensible overall limit.
    let argv = read_cstring_vec(argv_ptr_ptr, MAX_NR_STRING_ARGS, MAX_LEN_STRING_ARG, ctx)?;
    let envp = read_cstring_vec(envp_ptr_ptr, MAX_NR_STRING_ARGS, MAX_LEN_STRING_ARG, ctx)?;

    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();

    debug!(
        "file path: {:?}, argv = {:?}, envp = {:?}",
        path_resolver.make_abs_path(&elf_file).into_string(),
        argv,
        envp
    );

    let program_to_load =
        ProgramToLoad::build_from_file(elf_file.clone(), &path_resolver, argv, envp)?;

    let new_vmar = VmarHandle::new(ProcessVm::new(elf_file.clone()));
    let elf_load_info = program_to_load.load_to_vmar(&new_vmar, &path_resolver)?;

    // Ensure no other thread is concurrently performing exit_group or execve.
    // If such an operation is in progress, return EAGAIN.
    let mut task_set = ctx.process.tasks().lock();
    if task_set.has_exited_group() || task_set.in_execve() {
        return_errno_with_message!(
            Errno::EAGAIN,
            "the process has exited or has already executed a new program"
        );
    }
    task_set.start_execve();

    // Terminate all other threads
    sigkill_other_threads(ctx.task, &task_set);
    drop(task_set);

    let former_tid = ctx.posix_thread.tid();

    // After this point, failures in subsequent operations are fatal: the process
    // state may be left inconsistent and it can never return to user mode.

    let res = do_execve_no_return(
        ctx,
        user_context,
        elf_file,
        thread_name,
        new_vmar,
        &elf_load_info,
    );

    if res.is_ok() {
        ctx.posix_thread
            .ptrace_may_stop_on(PtraceEvent::Exec(former_tid), ctx, user_context);
    } else {
        ctx.posix_thread
            .enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
    }

    ctx.process.tasks().lock().finish_execve();

    res
}

fn read_cstring_vec(
    array_ptr: Vaddr,
    max_string_number: usize,
    max_string_len: usize,
    ctx: &Context,
) -> Result<Vec<CString>> {
    // On Linux, argv pointer and envp pointer can be specified as NULL.
    if array_ptr == 0 {
        return Ok(Vec::new());
    }

    let mut res = Vec::new();
    let mut read_addr = array_ptr;

    let user_space = ctx.user_space();
    for _ in 0..max_string_number {
        let cstring_ptr = user_space.read_val::<usize>(read_addr)?;
        read_addr += 8;

        if cstring_ptr == 0 {
            return Ok(res);
        }

        let cstring = user_space
            .read_cstring(cstring_ptr, max_string_len)
            .map_err(|err| {
                if err.error() == Errno::ENAMETOOLONG {
                    Error::with_message(Errno::E2BIG, "there are too many bytes in the argument")
                } else {
                    err
                }
            })?;
        res.push(cstring);
    }

    return_errno_with_message!(Errno::E2BIG, "there are too many arguments");
}

fn do_execve_no_return(
    ctx: &Context,
    user_context: &mut UserContext,
    elf_file: Path,
    thread_name: ThreadName,
    new_vmar: VmarHandle,
    elf_load_info: &ElfLoadInfo,
) -> Result<()> {
    let Context {
        process,
        thread_local,
        posix_thread,
        ..
    } = ctx;

    // Wait for all other threads to terminate,
    // then promote the current thread to be the process's main thread if necessary.
    wait_other_threads_exit(ctx)?;
    make_current_main_thread(ctx);

    // Activate the new VMAR in the current context and apply file-capability changes,
    // while holding the process VMAR lock.
    // This prevents race conditions when checking access permissions while opening
    // `/proc/[pid]/mem` or `/proc/[pid]/maps`.
    let (vmar_guard, old_vmar) = activate_vmar(ctx, new_vmar);
    apply_caps_from_exec(process, ctx.credentials_mut(), elf_file.inode())?;
    drop(vmar_guard);
    drop(old_vmar);

    // After the program has been successfully loaded, the virtual memory of the current process
    // is initialized. Hence, it is necessary to clear the previously recorded robust list.
    *thread_local.robust_list().borrow_mut() = None;
    thread_local.clear_child_tid().set(0);

    // Set up the CPU context.
    set_cpu_context(thread_local, user_context, elf_load_info);

    // If this was a vfork child, reset vfork-specific state.
    reset_vfork_child(process);

    // Unshare file descriptor table and close files with O_CLOEXEC flag.
    unshare_and_close_files(ctx);

    // Set the thread name.
    *posix_thread.thread_name().lock() = thread_name;

    // Unshare and reset signal dispositions to their default actions.
    unshare_and_reset_sigdispositions(process);
    // Reset the alternate signal stack to its default state.
    *thread_local.sig_stack().borrow_mut() = SigStack::default();
    // Restore the process exit signal to SIGCHLD.
    process.set_exit_signal(SIGCHLD);

    Ok(())
}

fn wait_other_threads_exit(ctx: &Context) -> Result<()> {
    let is_main_thread = ctx.posix_thread.tid() == ctx.process.pid();

    let mut tasks = ctx.process.tasks().lock();
    loop {
        if is_main_thread {
            if tasks.as_slice().len() == 1 {
                return Ok(());
            }
        } else if tasks.as_slice().len() == 2 && tasks.has_exited_main() {
            return Ok(());
        }

        // Wait until any signal comes or any other thread exits.
        let (waiter, waker) = Waiter::new_pair();

        ctx.posix_thread
            .set_signalled_waker(waker.clone(), PauseReason::Sleep);
        if ctx.has_pending_sigkill() {
            ctx.posix_thread.clear_signalled_waker();
            return_errno_with_message!(Errno::EAGAIN, "the current thread has received SIGKILL");
        }

        tasks.set_execve_waker(waker);
        drop(tasks);

        waiter.wait();

        ctx.posix_thread.clear_signalled_waker();

        tasks = ctx.process.tasks().lock();
        tasks.clear_execve_waker();
    }
}

fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();
    let old_tid = ctx.posix_thread.tid();

    // The current thread is already the main thread.
    if old_tid == pid {
        return;
    }

    // The current thread is not the main thread.

    // Lock order: PID table -> tracer.tracees -> tasks of process
    let mut pid_table = pid_table::pid_table_mut();
    let tracer = ctx.posix_thread.tracer();
    let mut tracer_tracees = tracer
        .as_ref()
        .map(|tracer| tracer.as_posix_thread().unwrap().tracees().unwrap().lock());
    let mut tasks = ctx.process.tasks().lock();

    assert!(tasks.has_exited_main());
    assert!(tasks.in_execve());
    assert_eq!(tasks.as_slice().len(), 2);
    assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

    tasks.swap_main();
    ctx.posix_thread.set_main(pid);

    if let Some(tracer_tracees) = tracer_tracees.as_mut()
        && let Some(tracee) = tracer_tracees.remove(&old_tid)
    {
        tracer_tracees.insert(pid, tracee);
    }

    drop(tasks);
    drop(tracer_tracees);

    let thread = pid_table.take_thread(old_tid).unwrap();
    pid_table.replace_thread(pid, &thread);
}

fn set_cpu_context(
    thread_local: &ThreadLocal,
    user_context: &mut UserContext,
    elf_load_info: &ElfLoadInfo,
) {
    let supp = thread_local.supp_user_context();

    // Reset FPU context.
    supp.fpu().set(FpuContext::new());

    // Reset general-purpose registers.
    *user_context.general_regs_mut() = GeneralRegs::default();
    // Clear the TLS pointer.
    #[cfg(target_arch = "x86_64")]
    {
        supp.fs_base().set(FsBase::default());
        supp.gs_base().set(GsBase::default());
    }
    #[cfg(not(target_arch = "x86_64"))]
    user_context.set_tls_pointer(0);

    // Set the new instruction pointer to the ELF entry point.
    user_context.set_instruction_pointer(elf_load_info.entry_point as _);
    debug!("entry_point: 0x{:x}", elf_load_info.entry_point);
    // Set the new user-space stack pointer.
    user_context.set_stack_pointer(elf_load_info.user_stack_top as _);
    debug!("user stack top: 0x{:x}", elf_load_info.user_stack_top);
}

/// Sets the UID and GID in the credentials according to the ELF inode.
///
/// The capabilities will be updated accordingly.
fn apply_caps_from_exec(
    process: &Process,
    credentials: Credentials<ReadWriteOp>,
    elf_inode: &Arc<dyn Inode>,
) -> Result<()> {
    let mode = elf_inode.mode()?;
    let no_new_privs = credentials.no_new_privs();
    let set_uid = if mode.has_set_uid() && !no_new_privs {
        Some(elf_inode.owner()?)
    } else {
        None
    };
    let set_gid = if mode.has_set_gid() && !no_new_privs {
        Some(elf_inode.group()?)
    } else {
        None
    };

    // Clear the ambient capability set when executing a privileged file.
    // Currently, only setuid/setgid files are considered privileged.
    // TODO: Also clear ambient capabilities when executing files with file capabilities
    // (security.capability xattr) once file capabilities are supported.
    if set_uid.is_some() || set_gid.is_some() {
        credentials.clear_ambient_capset();
    }
    apply_set_uid(process, &credentials, set_uid);
    apply_set_gid(process, &credentials, set_gid);
    credentials.set_keep_capabilities(false)?;

    Ok(())
}

/// Applies the set-user-ID effect to the credentials.
///
/// If `set_uid` is `Some`, the effective UID is set to the given UID.
fn apply_set_uid(current: &Process, credentials: &Credentials<ReadWriteOp>, set_uid: Option<Uid>) {
    if let Some(owner) = set_uid {
        credentials.set_euid(owner);

        current.clear_parent_death_signal();
    }

    // No matter whether the file has the set-user-ID bit, SUID should be reset.
    credentials.reset_suid();
}

/// Applies the set-group-ID effect to the credentials.
///
/// If `set_gid` is `Some`, the effective GID is set to the given GID.
fn apply_set_gid(current: &Process, credentials: &Credentials<ReadWriteOp>, set_gid: Option<Gid>) {
    if let Some(group) = set_gid {
        credentials.set_egid(group);

        current.clear_parent_death_signal();
    }

    // No matter whether the file has the set-group-ID bit, SGID should be reset.
    credentials.reset_sgid();
}

fn reset_vfork_child(process: &Process) {
    if process.status().is_vfork_child() {
        // Resumes the parent process.
        process.status().set_vfork_child(false);
        let parent = process.parent().lock().process().upgrade().unwrap();
        parent.children_wait_queue().wake_all();
    }
}

fn unshare_and_close_files(ctx: &Context) {
    ctx.unshare_files();

    ctx.thread_local
        .borrow_file_table()
        .unwrap()
        .write()
        .close_files_on_exec();
}

fn unshare_and_reset_sigdispositions(process: &Process) {
    let mut sig_dispositions = process.sig_dispositions().lock();

    let mut new = *sig_dispositions.lock();
    new.inherit();

    *sig_dispositions = Arc::new(Mutex::new(new));
}
