// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::UserContext;

use crate::cpu::LinuxAbi;

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.user_context.get_syscall_num()
    }

    fn syscall_ret(&self) -> usize {
        self.user_context.get_syscall_ret()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.user_context.set_syscall_ret(ret)
    }

    fn syscall_args(&self) -> [usize; 6] {
        self.user_context.get_syscall_args()
    }

    fn set_tls_pointer(&mut self, tls: usize) {
        self.set_tp(tls);
    }

    fn tls_pointer(&self) -> usize {
        self.tp()
    }
}
