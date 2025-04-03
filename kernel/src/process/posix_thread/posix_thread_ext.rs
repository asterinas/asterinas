// SPDX-License-Identifier: MPL-2.0

use ostd::{cpu::context::UserContext, task::Task, user::UserContextApi};

use super::{builder::PosixThreadBuilder, name::ThreadName, PosixThread};
use crate::{
    fs::{
        fs_resolver::{FsPath, AT_FDCWD},
        thread_info::ThreadFsInfo,
    },
    prelude::*,
    process::{process_vm::ProcessVm, program_loader::ProgramToLoad, Credentials, Process},
    thread::{AsThread, Thread, Tid},
};

/// A trait to provide the `as_posix_thread` method for tasks and threads.
pub trait AsPosixThread {
    /// Returns the associated [`PosixThread`].
    fn as_posix_thread(&self) -> Option<&PosixThread>;
}

impl AsPosixThread for Thread {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.data().downcast_ref::<PosixThread>()
    }
}

impl AsPosixThread for Task {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.as_thread()?.as_posix_thread()
    }
}

/// Creates a task for running an executable file.
///
/// This function should _only_ be used to create the init user task.
pub fn create_posix_task_from_executable(
    tid: Tid,
    credentials: Credentials,
    process_vm: &ProcessVm,
    executable_path: &str,
    process: Weak<Process>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Task>> {
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

    let mut user_ctx = UserContext::default();
    user_ctx.set_instruction_pointer(elf_load_info.entry_point() as _);
    user_ctx.set_stack_pointer(elf_load_info.user_stack_top() as _);
    let thread_name = Some(ThreadName::new_from_executable_path(executable_path)?);
    let thread_builder = PosixThreadBuilder::new(tid, Arc::new(user_ctx), credentials)
        .thread_name(thread_name)
        .process(process)
        .fs(Arc::new(fs));
    Ok(thread_builder.build())
}
