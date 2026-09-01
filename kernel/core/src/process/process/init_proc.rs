// SPDX-License-Identifier: MPL-2.0

//! This module defines functions related to spawning the init process.

use ostd::{arch::cpu::context::UserContext, task::Task, user::UserContextApi};

use super::{Process, Session};
use crate::{
    fs::{
        thread_info::ThreadFsInfo,
        vfs::path::{Path, PathResolver},
    },
    prelude::*,
    process::{
        Credentials, ProcessVm, ShebangScriptPath, UndetectedExecutable, UserNamespace,
        pid_table::{self, PidReservation},
        posix_thread::{PosixThreadBuilder, derive_thread_name},
        program_loader::ProgramToLoad,
        rlimit::new_resource_limits_for_init,
        signal::sig_disposition::SigDispositions,
    },
    sched::Nice,
    vm::vmar::VmarHandle,
};

/// Creates and schedules the init process to run.
pub(crate) fn spawn_init_process(
    path_resolver: PathResolver,
    executable_path: Path,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    let (process, reservation) = create_init_process(path_resolver, executable_path, argv, envp)?;

    // Linux starts the init process without placing it in a process group or session.
    // It joins one only after userspace first calls `setsid()`.
    //
    // Asterinas instead requires every process to belong to both a process group and
    // a session. The init process is therefore placed in a bootstrap process group
    // and session with PGID and SID set to zero. This preserves the same user-visible
    // behavior.
    set_bootstrap_session_and_group(&process, reservation);

    process.run();

    Ok(process)
}

fn create_init_process(
    path_resolver: PathResolver,
    executable_path: Path,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<(Arc<Process>, PidReservation)> {
    let fs = ThreadFsInfo::new(path_resolver);

    let reservation = pid_table::reserve_tid()?;
    let pid = reservation.tid();
    let pid_entry = reservation.pid_entry();
    let vmar = VmarHandle::new(ProcessVm::new(executable_path.clone()));
    let resource_limits = new_resource_limits_for_init();
    let nice = Nice::default();
    let oom_score_adj = 0;
    let sig_dispositions = Arc::new(Mutex::new(SigDispositions::default()));
    let user_ns = UserNamespace::get_init_singleton().clone();

    let init_proc = Process::new(
        pid,
        vmar.clone_arc(),
        resource_limits,
        nice,
        oom_score_adj,
        sig_dispositions,
        user_ns,
    );

    let init_task = create_init_task(pid_entry, &init_proc, fs, vmar, executable_path, argv, envp)?;
    init_proc.tasks().lock().insert(init_task).unwrap();

    Ok((init_proc, reservation))
}

fn set_bootstrap_session_and_group(process: &Arc<Process>, reservation: PidReservation) {
    // Locking order: PID table -> process group
    let mut pid_table = pid_table::pid_table_mut();

    // Add the process to the bootstrap process group and session
    let (session, process_group) = Session::new_bootstrap_pair(process);
    pid_table.insert_session(session.sid(), &session);
    pid_table.insert_process_group(process_group.pgid(), &process_group);
    *process.process_group.lock() = Some(process_group);

    // Add the new process to the global table and commit its reserved PID.
    reservation.commit_process(&mut pid_table, process);
}

/// Creates the init task from the given executable path.
fn create_init_task(
    pid_entry: Arc<pid_table::PidEntry>,
    process: &Arc<Process>,
    fs: ThreadFsInfo,
    vmar: VmarHandle,
    executable_path: Path,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Task>> {
    let credentials = Credentials::new_root();

    let (elf_load_info, executable_abs_path) = {
        let path_resolver = fs.resolver().read();
        let executable_abs_path = path_resolver.make_abs_path(&executable_path).into_string();
        let shebang_script_path =
            ShebangScriptPath::Accessible(CString::new(executable_abs_path.clone()).unwrap());

        let executable = UndetectedExecutable::open(executable_path.clone(), shebang_script_path)?;
        let program_to_load =
            ProgramToLoad::from_executable(executable, &path_resolver, argv, envp)?;
        let vmar = process.lock_vmar();
        let elf_load_info = program_to_load.load_to_vmar(vmar.unwrap(), &path_resolver)?;

        (elf_load_info, executable_abs_path)
    };

    let mut user_ctx = UserContext::default();
    user_ctx.set_instruction_pointer(elf_load_info.entry_point as _);
    user_ctx.set_stack_pointer(elf_load_info.user_stack_top as _);

    let thread_name = derive_thread_name(&executable_abs_path);

    let thread_builder = PosixThreadBuilder::new(
        pid_entry,
        thread_name,
        Box::new(user_ctx),
        credentials,
        vmar,
    )
    .process(Arc::downgrade(process))
    .fs(Arc::new(fs));
    Ok(thread_builder.build())
}
