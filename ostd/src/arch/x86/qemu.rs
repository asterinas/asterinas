// SPDX-License-Identifier: MPL-2.0

//! Provides the ability to exit QEMU and return a value as debug result.

/// The exit code of x86 QEMU isa debug device.
///
/// In `qemu-system-x86_64` the exit code will be `(code << 1) | 1`. So you
/// could never let QEMU invoke `exit(0)`. We also need to check if the exit
/// code is returned by the kernel, so we couldn't use 0 as exit_success
/// because this may conflict with QEMU return value 1, which indicates that
/// QEMU itself fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    /// The code that indicates a successful exit.
    Success = 0x10,
    /// The code that indicates a failed exit.
    Failed = 0x20,
}

/// Exits QEMU with the given exit code.
///
/// This function assumes that the kernel is run in QEMU with the following
/// QEMU command line arguments that specifies the ISA debug exit device:
/// `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;
    let mut port = Port::new(0xf4);

    // SAFETY: The write to the ISA debug exit port is safe and `0xf4` should
    // be the port number.
    unsafe {
        port.write(exit_code as u32);
    }
    unreachable!()
}
