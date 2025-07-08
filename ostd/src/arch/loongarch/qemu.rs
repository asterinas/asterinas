// SPDX-License-Identifier: MPL-2.0

//! Providing the ability to exit QEMU and return a value as debug result.

/// The exit code of QEMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuExitCode {
    /// The code that indicates a successful exit.
    Success,
    /// The code that indicates a failed exit.
    Failed,
}

/// Exits QEMU with the given exit code.
//  FIXME: Support the transfer of the exit code to QEMU.
pub fn exit_qemu(_exit_code: QemuExitCode) -> ! {
    // The exit address is acquired from the device tree, and
    // is mapped by DMW2.
    // FIXME: Because a panic occurring before device tree parsing may cause a deadlock,
    // a fixed value is temporarily used here
    const EXIT_ADDR: *mut u8 = 0x8000_0000_100e_001c as *mut u8;
    // The exit value is acquired from the device tree.
    // FIXME: Because a panic occurring before device tree parsing may cause a deadlock,
    // a fixed value is temporarily used here
    const EXIT_VALUE: u8 = 0x34;

    // SAFETY: The write to the ISA debug exit mapped address is safe.
    unsafe {
        core::ptr::write_volatile(EXIT_ADDR, EXIT_VALUE);
    }
    unreachable!("Qemu does not exit");
}
