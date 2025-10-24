// SPDX-License-Identifier: MPL-2.0

//! This module defines functions related to spawning the init process.

use ostd::{arch::cpu::context::UserContext, task::Task, user::UserContextApi};

use super::Process;
use crate::{
    fs::{
        fs_resolver::{FsItem, FsPath},
        path::MountNamespace,
        thread_info::ThreadFsInfo,
    },
    prelude::*,
    process::{
        posix_thread::{allocate_posix_tid, PosixThreadBuilder, ThreadName},
        process_table,
        process_vm::ProcessVm,
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
    let process_vm = ProcessVm::alloc();
    let resource_limits = ResourceLimits::default();
    let nice = Nice::default();
    let oom_score_adj = 0;
    let sig_dispositions = Arc::new(Mutex::new(SigDispositions::default()));
    let user_ns = UserNamespace::get_init_singleton().clone();

    let fs = {
        let fs_resolver = MountNamespace::get_init_singleton().new_fs_resolver();
        ThreadFsInfo::new(fs_resolver)
    };
    let fs_path = FsPath::try_from(executable_path)?;
    let elf_file = FsItem::Real(fs.resolver().read().lookup(&fs_path)?);

    let init_proc = Process::new(
        pid,
        elf_file.clone(),
        process_vm,
        resource_limits,
        nice,
        oom_score_adj,
        sig_dispositions,
        user_ns,
    );

    let init_task = create_init_task(
        pid,
        init_proc.vm(),
        fs,
        &elf_file,
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
    fs: ThreadFsInfo,
    elf_file: &FsItem,
    process: Weak<Process>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Task>> {
    let credentials = Credentials::new_root();

    let elf_load_info = {
        let fs_resolver = fs.resolver().read();
        let program_to_load = ProgramToLoad::build_from_inode(
            elf_file.inode().unwrap(),
            &fs_resolver,
            argv,
            envp,
            1,
        )?;
        process_vm.clear_and_map();
        program_to_load.load_to_vm(process_vm, &fs_resolver)?
    };

    let mut user_ctx = UserContext::default();
    user_ctx.set_instruction_pointer(elf_load_info.entry_point as _);
    user_ctx.set_stack_pointer(elf_load_info.user_stack_top as _);
    let thread_name = ThreadName::new_from_executable_path(&elf_file.display_name());
    let thread_builder = PosixThreadBuilder::new(tid, thread_name, Box::new(user_ctx), credentials)
        .process(process)
        .fs(Arc::new(fs))
        .is_init_process();
    Ok(thread_builder.build())
}
