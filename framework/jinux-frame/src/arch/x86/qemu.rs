//! QEMU isa debug device.

/// The exit code of x86 QEMU isa debug device. In `qemu-system-x86_64` the
/// exit code will be `(code << 1) | 1`. So you could never let QEMU invoke
/// `exit(0)`. We also need to check if the exit code is returned by the
/// kernel, so we couldn't use 0 as exit_success because this may conflict
/// with QEMU return value 1, which indicates that QEMU itself fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x20,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    unreachable!()
}
