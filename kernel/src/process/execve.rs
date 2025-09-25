// SPDX-License-Identifier: MPL-2.0

use aster_rights::WriteOp;
use ostd::{
    arch::cpu::context::{FpuContext, GeneralRegs, UserContext},
    task::Task,
    user::UserContextApi,
};

use crate::{
    fs::{fs_resolver::FsResolver, path::Path},
    prelude::*,
    process::{
        posix_thread::{sigkill_other_threads, thread_table, PosixThread, ThreadLocal, ThreadName},
        process_vm::{renew_vmar_and_map_heap, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS},
        program_loader::elf::ElfLoadInfo,
        signal::{
            constants::{SIGCHLD, SIGKILL},
            signals::kernel::KernelSignal,
            SigStack,
        },
        ContextUnshareAdminApi, Credentials, Process, ProgramToLoad,
    },
    thread::{AsThread, Thread},
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
    let program_to_load =
        ProgramToLoad::build_from_file(elf_file.clone(), &fs_resolver, argv, envp, 1)?;

    // Ensure no other thread is concurrently performing exit_group or execve.
    // If such an operation is in progress, return EAGAIN.
    let mut task_set = ctx.process.tasks().lock();
    if task_set.has_exited_group() || task_set.in_execve() {
        return_errno_with_message!(
            Errno::EAGAIN,
            "an exit_group or another execve is already in progress"
        );
    }
    task_set.set_in_execve();

    // Terminate all other threads
    sigkill_other_threads(ctx.task, &task_set);
    drop(task_set);

    // After this point, failures in subsequent operations are fatal: the process
    // state may be left inconsistent and it can never to return to user mode.

    let res = do_execve_no_return(ctx, user_context, &elf_file, &fs_resolver, program_to_load);

    if res.is_err() {
        ctx.posix_thread
            .enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
    }

    ctx.process.tasks().lock().reset_in_execve();

    res
}

fn do_execve_no_return(
    ctx: &Context,
    user_context: &mut UserContext,
    elf_file: &Path,
    fs_resolver: &FsResolver,
    program_to_load: ProgramToLoad,
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

    // Reset the virtual memory state.
    renew_vmar_and_map_heap(ctx);
    // After the program has been successfully loaded, the virtual memory of the current process
    // is initialized. Hence, it is necessary to clear the previously recorded robust list.
    *thread_local.robust_list().borrow_mut() = None;
    thread_local.clear_child_tid().set(0);

    // Load the binary into the process's address space and set up the CPU context.
    let elf_load_info = program_to_load.load_to_vm(process.vm(), fs_resolver)?;
    set_cpu_context(thread_local, user_context, &elf_load_info);

    // Apply file-capability changes.
    apply_caps_from_exec(process, posix_thread, elf_file)?;

    // If this was a vfork child, reset vfork-specific state.
    reset_vfork_child(process);

    // Unshare file descriptor table and close files with O_CLOEXEC flag.
    unshare_and_close_files(ctx);

    // Update the process's executable path and set the thread name
    let executable_path = elf_file.abs_path();
    *posix_thread.thread_name().lock() = ThreadName::new_from_executable_path(&executable_path);
    process.set_executable_path(executable_path);

    // Reset signal dispositions to their default actions.
    process.sig_dispositions().lock().inherit();
    // Reset the alternate signal stack to its default state.
    *thread_local.sig_stack().borrow_mut() = SigStack::default();
    // Restore the process exit signal to SIGCHLD.
    process.set_exit_signal(SIGCHLD);

    Ok(())
}

fn wait_other_threads_exit(ctx: &Context) -> Result<()> {
    let is_main_thread = ctx.posix_thread.tid() == ctx.process.pid();
    let expected_count = if is_main_thread { 1 } else { 2 };

    loop {
        let tasks = ctx.process.tasks().lock();
        if ctx.posix_thread.has_pending_sigkill() {
            return_errno_with_message!(Errno::EAGAIN, "the current thread has received SIGKILL");
        }

        if tasks.as_slice().len() == expected_count {
            if is_main_thread {
                return Ok(());
            }

            let main_thread = tasks.main();
            if main_thread.as_thread().unwrap().is_exited() {
                return Ok(());
            }
        }

        drop(tasks);
        Task::yield_now();
    }
}

fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();

    // The current thread is already the main thread.
    if ctx.posix_thread.tid() == pid {
        return;
    }

    // The current thread is not the main thread.
    let mut thread_table = thread_table::lock();
    let mut tasks = ctx.process.tasks().lock();
    tasks.set_main(ctx);
    ctx.posix_thread.set_main(pid);
    assert!(thread_table.remove(&pid).is_some());
    thread_table.insert(pid, Thread::current().unwrap());
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

fn apply_caps_from_exec(
    process: &Process,
    posix_thread: &PosixThread,
    elf_file: &Path,
) -> Result<()> {
    // FIXME: We need to recalculate the capabilities during execve even the executable file
    // does not have setuid/setgid bit.
    let credentials = posix_thread.credentials_mut();
    set_uid_from_elf(process, &credentials, elf_file)?;
    set_gid_from_elf(process, &credentials, elf_file)?;
    credentials.set_keep_capabilities(false);

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

/// Sets uid for credentials as the same of uid of elf file if elf file has `set_uid` bit.
fn set_uid_from_elf(
    current: &Process,
    credentials: &Credentials<WriteOp>,
    elf_file: &Path,
) -> Result<()> {
    if elf_file.mode()?.has_set_uid() {
        let uid = elf_file.owner()?;
        credentials.set_euid(uid);

        current.clear_parent_death_signal();
    }

    // No matter whether the elf_file has `set_uid` bit, suid should be reset.
    credentials.reset_suid();
    Ok(())
}

/// Sets gid for credentials as the same of gid of elf file if elf file has `set_gid` bit.
fn set_gid_from_elf(
    current: &Process,
    credentials: &Credentials<WriteOp>,
    elf_file: &Path,
) -> Result<()> {
    if elf_file.mode()?.has_set_gid() {
        let gid = elf_file.group()?;
        credentials.set_egid(gid);

        current.clear_parent_death_signal();
    }

    // No matter whether the the elf file has `set_gid` bit, sgid should be reset.
    credentials.reset_sgid();
    Ok(())
}
