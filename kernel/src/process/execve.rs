// SPDX-License-Identifier: MPL-2.0

use aster_rights::WriteOp;
use ostd::{
    arch::cpu::context::{FpuContext, GeneralRegs, UserContext},
    mm::VmIo,
    sync::Waiter,
    user::UserContextApi,
};

use super::process_vm::activate_vmar;
use crate::{
    fs::{path::Path, utils::Inode},
    prelude::*,
    process::{
        ContextUnshareAdminApi, Credentials, Process,
        posix_thread::{PosixThread, ThreadLocal, ThreadName, sigkill_other_threads, thread_table},
        process_vm::{MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS, ProcessVm},
        program_loader::{ProgramToLoad, elf::ElfLoadInfo},
        signal::{
            HandlePendingSignal, PauseReason, SigStack,
            constants::{SIGCHLD, SIGKILL},
            signals::kernel::KernelSignal,
        },
    },
    vm::vmar::Vmar,
};

pub fn do_execve(
    elf_file: Path,
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
    debug!(
        "filename: {:?}, argv = {:?}, envp = {:?}",
        elf_file.abs_path(),
        argv,
        envp
    );

    let fs_ref = ctx.thread_local.borrow_fs();
    let fs_resolver = fs_ref.resolver().read();

    let elf_inode = elf_file.inode();
    let program_to_load =
        ProgramToLoad::build_from_inode(elf_inode.clone(), &fs_resolver, argv, envp)?;

    let new_vmar = Vmar::new(ProcessVm::new(elf_file.clone()));
    let elf_load_info = program_to_load.load_to_vmar(new_vmar.as_ref(), &fs_resolver)?;

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

    // After this point, failures in subsequent operations are fatal: the process
    // state may be left inconsistent and it can never return to user mode.

    let res = do_execve_no_return(ctx, user_context, elf_file, new_vmar, &elf_load_info);

    if res.is_err() {
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
    new_vmar: Arc<Vmar>,
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
    thread_table::make_current_main_thread(ctx);

    // Activate the new VMAR, where the ELF has been loaded, in the current context.
    activate_vmar(ctx, new_vmar);

    // After the program has been successfully loaded, the virtual memory of the current process
    // is initialized. Hence, it is necessary to clear the previously recorded robust list.
    *thread_local.robust_list().borrow_mut() = None;
    thread_local.clear_child_tid().set(0);

    // Set up the CPU context.
    set_cpu_context(thread_local, user_context, elf_load_info);

    // Apply file-capability changes.
    apply_caps_from_exec(process, posix_thread, elf_file.inode())?;

    // If this was a vfork child, reset vfork-specific state.
    reset_vfork_child(process);

    // Unshare file descriptor table and close files with O_CLOEXEC flag.
    unshare_and_close_files(ctx);

    // Update the process's executable path and set the thread name
    let executable_path = elf_file.abs_path();
    *posix_thread.thread_name().lock() = ThreadName::new_from_executable_path(&executable_path);

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

fn set_cpu_context(
    thread_local: &ThreadLocal,
    user_context: &mut UserContext,
    elf_load_info: &ElfLoadInfo,
) {
    // Reset FPU context.
    thread_local.fpu().set_context(FpuContext::new());

    // Reset general-purpose registers.
    *user_context.general_regs_mut() = GeneralRegs::default();
    // Clear the TLS pointer.
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
    posix_thread: &PosixThread,
    elf_inode: &Arc<dyn Inode>,
) -> Result<()> {
    // FIXME: We need to recalculate the capabilities during execve even the executable inode
    // does not have setuid/setgid bit.
    let credentials = posix_thread.credentials_mut();
    set_uid_from_elf(process, &credentials, elf_inode)?;
    set_gid_from_elf(process, &credentials, elf_inode)?;
    credentials.set_keep_capabilities(false)?;

    Ok(())
}

/// Sets the UID in the credentials according to the ELF inode.
///
/// If the ELF inode has the `set_uid` bit, the effective UID is set to the same value as the ELF
/// inode's UID.
fn set_uid_from_elf(
    current: &Process,
    credentials: &Credentials<WriteOp>,
    elf_inode: &Arc<dyn Inode>,
) -> Result<()> {
    if elf_inode.mode()?.has_set_uid() {
        let uid = elf_inode.owner()?;
        credentials.set_euid(uid);

        current.clear_parent_death_signal();
    }

    // No matter whether the ELF inode has `set_uid` bit, SUID should be reset.
    credentials.reset_suid();
    Ok(())
}

/// Sets the GID in the credentials according to the ELF inode.
///
/// If the ELF inode has the `set_gid` bit, the effective GID is set to the same value as the ELF
/// inode's GID.
fn set_gid_from_elf(
    current: &Process,
    credentials: &Credentials<WriteOp>,
    elf_inode: &Arc<dyn Inode>,
) -> Result<()> {
    if elf_inode.mode()?.has_set_gid() {
        let gid = elf_inode.group()?;
        credentials.set_egid(gid);

        current.clear_parent_death_signal();
    }

    // No matter whether the ELF inode has `set_gid` bit, SGID should be reset.
    credentials.reset_sgid();
    Ok(())
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
