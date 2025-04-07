// SPDX-License-Identifier: MPL-2.0

//! This module defines functions related to spawning the init process.

use ostd::{cpu::context::UserContext, task::Task, user::UserContextApi};

use super::{Process, Terminal};
use crate::{
    fs::{
        fs_resolver::{FsPath, AT_FDCWD},
        thread_info::ThreadFsInfo,
    },
    prelude::*,
    process::{
        posix_thread::{allocate_posix_tid, PosixThreadBuilder, ThreadName},
        process_table,
        process_vm::ProcessVm,
        rlimit::ResourceLimits,
        signal::sig_disposition::SigDispositions,
        Credentials, ProgramToLoad,
    },
    sched::Nice,
    thread::Tid,
};

/// Creates and schedules the init process to run.
pub fn spawn_init_process(
    executable_path: &str,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    // Ensure the path for init process executable is absolute.
    debug_assert!(executable_path.starts_with('/'));

    let process = create_init_process(executable_path, argv, envp)?;

    set_session_and_group(&process);

    // FIXME: This should be done by the userspace init process.
    (crate::device::tty::system_console().clone() as Arc<dyn Terminal>).set_control(&process)?;

    process.run();

    Ok(process)
}

fn create_init_process(
    executable_path: &str,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    let pid = allocate_posix_tid();
    let parent = Weak::new();
    let process_vm = ProcessVm::alloc();
    let resource_limits = ResourceLimits::default();
    let nice = Nice::default();
    let sig_dispositions = Arc::new(Mutex::new(SigDispositions::default()));

    let init_proc = Process::new(
        pid,
        parent,
        executable_path.to_string(),
        process_vm,
        resource_limits,
        nice,
        sig_dispositions,
    );

    let init_task = create_init_task(
        pid,
        init_proc.vm(),
        executable_path,
        Arc::downgrade(&init_proc),
        argv,
        envp,
    )?;
    init_proc.tasks().lock().insert(init_task).unwrap();

    Ok(init_proc)
}

fn set_session_and_group(process: &Arc<Process>) {
    // Locking order: session table -> group table -> process table -> process group
    let mut session_table_mut = process_table::session_table_mut();
    let mut group_table_mut = process_table::group_table_mut();
    let mut process_table_mut = process_table::process_table_mut();

    // Create a new process group and session for the process
    process.set_new_session(
        &mut process.process_group.lock(),
        &mut session_table_mut,
        &mut group_table_mut,
    );

    // Add the new process to the global table
    process_table_mut.insert(process.pid(), process.clone());
}

/// Creates the init task from the given executable file.
fn create_init_task(
    tid: Tid,
    process_vm: &ProcessVm,
    executable_path: &str,
    process: Weak<Process>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Task>> {
    let credentials = Credentials::new_root();
    let fs = ThreadFsInfo::default();
    let (_, elf_load_info) = {
        let fs_resolver = fs.resolver().read();
        let fs_path = FsPath::new(AT_FDCWD, executable_path)?;
        let elf_file = fs.resolver().read().lookup(&fs_path)?;
        let program_to_load =
            ProgramToLoad::build_from_file(elf_file, &fs_resolver, argv, envp, 1)?;
        process_vm.clear();
        program_to_load.load_to_vm(process_vm, &fs_resolver)?
    };

    let mut user_ctx = UserContext::default();
    user_ctx.set_instruction_pointer(elf_load_info.entry_point as _);
    user_ctx.set_stack_pointer(elf_load_info.user_stack_top as _);
    let thread_name = Some(ThreadName::new_from_executable_path(executable_path)?);
    let thread_builder = PosixThreadBuilder::new(tid, Arc::new(user_ctx), credentials)
        .thread_name(thread_name)
        .process(process)
        .fs(Arc::new(fs));
    Ok(thread_builder.build())
}
