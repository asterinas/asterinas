// SPDX-License-Identifier: MPL-2.0

//! This module defines functions related to spawning the init process.

use ostd::{cpu::context::UserContext, task::Task, user::UserContextApi};

use super::{Process, Terminal};
use crate::{
    device::tty::get_n_tty,
    fs::{
        fs_resolver::{FsPath, AT_FDCWD},
        thread_info::ThreadFsInfo,
    },
    prelude::*,
    process::{
        get_root_pid_namespace,
        pid_namespace::{NestedId, NestedIdAttachmentWriteGuard},
        posix_thread::{PosixThreadBuilder, ThreadName},
        process_vm::ProcessVm,
        rlimit::ResourceLimits,
        signal::sig_disposition::SigDispositions,
        Credentials, PidNamespace, ProgramToLoad,
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

    let pid_ns = get_root_pid_namespace();
    let nested_id = pid_ns.allocate_nested_id();
    let attachment = pid_ns.get_attachment(&nested_id).unwrap();
    let mut attachment_guard = attachment.write();

    let process = create_init_process(
        executable_path,
        argv,
        envp,
        &nested_id,
        &pid_ns,
        &mut attachment_guard,
    )?;

    set_session_and_group(&process, &mut attachment_guard);

    open_ntty_as_controlling_terminal(&process)?;

    process.run();

    Ok(process)
}

fn create_init_process<'a, 'b>(
    executable_path: &str,
    argv: Vec<CString>,
    envp: Vec<CString>,
    nested_id: &NestedId,
    pid_ns: &Arc<PidNamespace>,
    attachment_guard: &'a mut NestedIdAttachmentWriteGuard<'b>,
) -> Result<Arc<Process>> {
    let pid = pid_ns.get_current_id(&nested_id).unwrap();
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
        pid_ns.clone(),
        nested_id.clone(),
    );

    let init_task = create_init_task(
        init_proc.vm(),
        executable_path,
        Arc::downgrade(&init_proc),
        argv,
        envp,
        pid,
        nested_id,
        attachment_guard,
    )?;
    init_proc.tasks().lock().insert(init_task).unwrap();

    Ok(init_proc)
}

fn set_session_and_group(
    process: &Arc<Process>,
    attachment_guard: &mut NestedIdAttachmentWriteGuard,
) {
    // Create a new process group and session for the process
    process.set_new_session(&mut process.process_group.lock(), attachment_guard);

    // Add the new process to the global table
    attachment_guard.attach_process(process.clone());
}

/// Creates the init task from the given executable file.
fn create_init_task<'a, 'b>(
    process_vm: &ProcessVm,
    executable_path: &str,
    process: Weak<Process>,
    argv: Vec<CString>,
    envp: Vec<CString>,
    tid: Tid,
    nested_id: &NestedId,
    attachment_guard: &'a mut NestedIdAttachmentWriteGuard<'b>,
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
    let thread_builder = PosixThreadBuilder::new(
        tid,
        nested_id.clone(),
        attachment_guard,
        Arc::new(user_ctx),
        credentials,
    )
    .thread_name(thread_name)
    .process(process)
    .fs(Arc::new(fs));
    Ok(thread_builder.build())
}

/// Opens `N_TTY` as the controlling terminal for the process.
fn open_ntty_as_controlling_terminal(process: &Process) -> Result<()> {
    let tty = get_n_tty();

    let session = &process.session().unwrap();
    let process_group = process.process_group().unwrap();

    session.set_terminal(|| {
        tty.job_control().set_session(session);
        Ok(tty.clone())
    })?;

    tty.job_control().set_foreground(Some(&process_group))?;

    Ok(())
}
