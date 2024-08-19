// SPDX-License-Identifier: MPL-2.0

pub trait LinuxAbi {
    /// Get number of syscall
    fn syscall_num(&self) -> usize;

    /// Get return value of syscall
    fn syscall_ret(&self) -> usize;

    /// Set return value of syscall
    fn set_syscall_ret(&mut self, ret: usize);

    /// Get syscall args
    fn syscall_args(&self) -> [usize; 6];

    /// Set thread-local storage pointer
    fn set_tls_pointer(&mut self, tls: usize);

    /// Get thread-local storage pointer
    fn tls_pointer(&self) -> usize;
}
