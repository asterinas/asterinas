// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::UserContext;

use crate::cpu::LinuxAbi;

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.rax()
    }

    fn syscall_ret(&self) -> usize {
        self.rax()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_rax(ret);
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.rdi(),
            self.rsi(),
            self.rdx(),
            self.r10(),
            self.r8(),
            self.r9(),
        ]
    }

    fn set_tls_pointer(&mut self, tls: usize) {
        self.set_fsbase(tls);
    }

    fn tls_pointer(&self) -> usize {
        self.fsbase()
    }
}
