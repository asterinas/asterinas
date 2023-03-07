use jinux_frame::{cpu::CpuContext, user::UserSpace};

use crate::{
    prelude::*,
    process::{elf::load_elf_to_root_vmar, Process},
    rights::Full,
    thread::{allocate_tid, Thread},
    vm::vmar::Vmar,
};

use super::{builder::PosixThreadBuilder, name::ThreadName, PosixThread};
pub trait PosixThreadExt {
    fn as_posix_thread(&self) -> Option<&PosixThread>;
    fn new_posix_thread_from_executable(
        root_vmar: &Vmar<Full>,
        elf_path: CString,
        elf_file_content: &'static [u8],
        process: Weak<Process>,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Arc<Self>;
}

impl PosixThreadExt for Thread {
    /// This function should only be called when launch shell()
    fn new_posix_thread_from_executable(
        root_vmar: &Vmar<Full>,
        elf_path: CString,
        elf_file_content: &'static [u8],
        process: Weak<Process>,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Arc<Self> {
        let elf_load_info = load_elf_to_root_vmar(elf_file_content, &root_vmar, argv, envp)
            .expect("Load Elf failed");
        let vm_space = root_vmar.vm_space().clone();
        let mut cpu_ctx = CpuContext::default();
        cpu_ctx.set_rip(elf_load_info.entry_point());
        cpu_ctx.set_rsp(elf_load_info.user_stack_top());
        let user_space = Arc::new(UserSpace::new(vm_space, cpu_ctx));
        let thread_name = Some(ThreadName::new_from_elf_path(&elf_path).unwrap());
        let tid = allocate_tid();
        let thread_builder = PosixThreadBuilder::new(tid, user_space)
            .thread_name(thread_name)
            .process(process);
        thread_builder.build()
    }

    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.data().downcast_ref::<PosixThread>()
    }
}
