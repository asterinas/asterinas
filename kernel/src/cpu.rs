// SPDX-License-Identifier: MPL-2.0

/// A trait that describes the Linux system call convention (ABI) for the user context.
pub trait LinuxAbi {
    /// Gets the system call number.
    fn syscall_num(&self) -> usize;

    /// Gets the return value of the system call.
    fn syscall_ret(&self) -> usize;

    /// Sets the system call number.
    fn set_syscall_num(&mut self, num: usize);

    /// Sets the return value of the system call.
    fn set_syscall_ret(&mut self, ret: usize);

    /// Gets the arguments of the system call.
    fn syscall_args(&self) -> [usize; 6];
}
