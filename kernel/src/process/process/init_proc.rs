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
        pid_namespace::{get_init_pid_namespace, AncestorNsPids, TASK_LIST_LOCK},
        posix_thread::{PosixThreadBuilder, ThreadName},
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
    (crate::device::tty::get_n_tty().clone() as Arc<dyn Terminal>).set_control(&process)?;

    process.run();

    Ok(process)
}

fn create_init_process(
    executable_path: &str,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    let pid_ns = get_init_pid_namespace();
    let ns_pids = pid_ns.allocate_ids();
    let pid = pid_ns.get_current_id(&ns_pids).unwrap();
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
        ns_pids.clone(),
        pid_ns,
    );

    let init_task = create_init_task(
        pid,
        init_proc.vm(),
        executable_path,
        Arc::downgrade(&init_proc),
        argv,
        envp,
        ns_pids,
    )?;
    init_proc.tasks().lock().insert(init_task).unwrap();

    Ok(init_proc)
}

fn set_session_and_group(process: &Arc<Process>) {
    let pid_entry = process
        .pid_namespace()
        .get_entry_by_ids(&process.ns_pids)
        .unwrap();

    // Lock order: task list -> group of process
    let mut task_list_guard = TASK_LIST_LOCK.lock();
    let mut pid_entry_guard = pid_entry.with_task_list_guard(&mut task_list_guard);

    let mut process_group_mut = process.process_group.lock();
    // Create a new process group and session for the process
    process.set_new_session(&mut process_group_mut, &mut pid_entry_guard);

    // Attach the new process and its main thread to the PID namespaces.
    pid_entry_guard.attach_thread(process.main_thread());
    pid_entry_guard.attach_process(process.clone());
}

/// Creates the init task from the given executable file.
fn create_init_task(
    tid: Tid,
    process_vm: &ProcessVm,
    executable_path: &str,
    process: Weak<Process>,
    argv: Vec<CString>,
    envp: Vec<CString>,
    ns_tids: AncestorNsPids,
) -> Result<Arc<Task>> {
    let credentials = Credentials::new_root();
    let fs = ThreadFsInfo::default();
    let (_, elf_load_info) = {
        let fs_resolver = fs.resolver().read();
        let fs_path = FsPath::new(AT_FDCWD, executable_path)?;
        let elf_file = fs.resolver().read().lookup(&fs_path)?;
        let program_to_load =
            ProgramToLoad::build_from_file(elf_file, &fs_resolver, argv, envp, 1)?;
        process_vm.clear_and_map();
        program_to_load.load_to_vm(process_vm, &fs_resolver)?
    };

    let user_ctx = {
        let mut ctx = UserContext::default();
        ctx.set_instruction_pointer(elf_load_info.entry_point() as _);
        ctx.set_stack_pointer(elf_load_info.user_stack_top() as _);
        ctx
    };
    let thread_name = Some(ThreadName::new_from_executable_path(executable_path)?);
    let thread_builder = PosixThreadBuilder::new(tid, Arc::new(user_ctx), credentials, ns_tids)
        .thread_name(thread_name)
        .process(process)
        .fs(Arc::new(fs));
    Ok(thread_builder.build())
}
