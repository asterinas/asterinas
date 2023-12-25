use aster_frame::{cpu::UserContext, user::UserSpace};

use crate::{
    fs::fs_resolver::{FsPath, FsResolver, AT_FDCWD},
    prelude::*,
    process::{process_vm::ProcessVm, program_loader::load_program_to_vm, Credentials, Process},
    thread::{Thread, Tid},
};

use super::{builder::PosixThreadBuilder, name::ThreadName, PosixThread};
pub trait PosixThreadExt {
    fn as_posix_thread(&self) -> Option<&PosixThread>;
    #[allow(clippy::too_many_arguments)]
    fn new_posix_thread_from_executable(
        tid: Tid,
        credentials: Credentials,
        process_vm: &ProcessVm,
        fs_resolver: &FsResolver,
        executable_path: &str,
        process: Weak<Process>,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Arc<Self>>;
}

impl PosixThreadExt for Thread {
    /// This function should only be called when launch shell()
    fn new_posix_thread_from_executable(
        tid: Tid,
        credentials: Credentials,
        process_vm: &ProcessVm,
        fs_resolver: &FsResolver,
        executable_path: &str,
        process: Weak<Process>,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Arc<Self>> {
        let elf_file = {
            let fs_path = FsPath::new(AT_FDCWD, executable_path)?;
            fs_resolver.lookup(&fs_path)?
        };
        let (_, elf_load_info) =
            load_program_to_vm(process_vm, elf_file, argv, envp, fs_resolver, 1)?;

        let vm_space = process_vm.root_vmar().vm_space().clone();
        let mut cpu_ctx = UserContext::default();
        cpu_ctx.set_rip(elf_load_info.entry_point() as _);
        cpu_ctx.set_rsp(elf_load_info.user_stack_top() as _);
        let user_space = Arc::new(UserSpace::new(vm_space, cpu_ctx));
        let thread_name = Some(ThreadName::new_from_executable_path(executable_path)?);
        let thread_builder = PosixThreadBuilder::new(tid, user_space, credentials)
            .thread_name(thread_name)
            .process(process);
        Ok(thread_builder.build())
    }

    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.data().downcast_ref::<PosixThread>()
    }
}
