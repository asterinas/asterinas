// SPDX-License-Identifier: MPL-2.0

//! This module defines functions related to spawning the init process.

use ostd::{arch::cpu::context::UserContext, task::Task, user::UserContextApi};

use super::Process;
use crate::{
    fs::{fs_resolver::FsPath, path::MountNamespace, thread_info::ThreadFsInfo},
    prelude::*,
    process::{
        posix_thread::{allocate_posix_tid, PosixThreadBuilder, ThreadName},
        process_table,
        process_vm::new_vmar_and_map,
        rlimit::ResourceLimits,
        signal::sig_disposition::SigDispositions,
        Credentials, ProgramToLoad, UserNamespace,
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

    process.run();

    Ok(process)
}

fn create_init_process(
    executable_path: &str,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    let pid = allocate_posix_tid();
    let process_vm = new_vmar_and_map();
    let resource_limits = ResourceLimits::default();
    let nice = Nice::default();
    let oom_score_adj = 0;
    let sig_dispositions = Arc::new(Mutex::new(SigDispositions::default()));
    let user_ns = UserNamespace::get_init_singleton().clone();

    let init_proc = Process::new(
        pid,
        executable_path.to_string(),
        process_vm,
        resource_limits,
        nice,
        oom_score_adj,
        sig_dispositions,
        user_ns,
    );

    let init_task = create_init_task(pid, &init_proc, executable_path, argv, envp)?;
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
    process: &Arc<Process>,
    executable_path: &str,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Task>> {
    let credentials = Credentials::new_root();
    let fs = {
        let fs_resolver = MountNamespace::get_init_singleton().new_fs_resolver();
        ThreadFsInfo::new(fs_resolver)
    };
    let elf_load_info = {
        let fs_resolver = fs.resolver().read();
        let fs_path = FsPath::try_from(executable_path)?;
        let elf_file = fs.resolver().read().lookup(&fs_path)?;
        let program_to_load =
            ProgramToLoad::build_from_file(elf_file, &fs_resolver, argv, envp, 1)?;
        let vmar = process.lock_vmar();
        program_to_load.load_to_vmar(vmar.unwrap(), &fs_resolver)?
    };

    let mut user_ctx = UserContext::default();
    user_ctx.set_instruction_pointer(elf_load_info.entry_point as _);
    user_ctx.set_stack_pointer(elf_load_info.user_stack_top as _);
    let thread_name = ThreadName::new_from_executable_path(executable_path);
    let thread_builder = PosixThreadBuilder::new(tid, thread_name, Box::new(user_ctx), credentials)
        .process(Arc::downgrade(process))
        .fs(Arc::new(fs))
        .is_init_process();
    Ok(thread_builder.build())
}
