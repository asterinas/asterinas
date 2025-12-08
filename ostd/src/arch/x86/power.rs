// SPDX-License-Identifier: MPL-2.0

//! Power management.

mod qemu_isa_debug {
    //! The isa-debug-exit device in QEMU.
    //!
    //! Reference: <https://elixir.bootlin.com/qemu/v10.1.2/source/hw/misc/debugexit.c>

    use spin::Once;

    use crate::{
        arch::device::io_port::WriteOnlyAccess,
        io::IoPort,
        power::{ExitCode, inject_poweroff_handler},
    };

    // For `qemu-system-x86_64`, the exit code will be `(code << 1) | 1`. So it is not possible to
    // let QEMU invoke `exit(0)`. We also need to check if the exit code is returned by the kernel,
    // so we cannot use `0` as `EXIT_SUCCESS` because it may conflict with QEMU's return value `1`,
    // which indicates that QEMU itself fails.
    const EXIT_SUCCESS: u32 = 0x10;
    const EXIT_FAILURE: u32 = 0x20;

    static DEBUG_EXIT_PORT: Once<IoPort<u32, WriteOnlyAccess>> = Once::new();

    fn try_exit_qemu(code: ExitCode) {
        let value = match code {
            ExitCode::Success => EXIT_SUCCESS,
            ExitCode::Failure => EXIT_FAILURE,
        };

        // If possible, keep this method panic-free because it may be called by the panic handler.
        if let Some(port) = DEBUG_EXIT_PORT.get() {
            port.write(value);
        }
    }

    pub(super) fn init() {
        const DEBUG_EXIT_PORT_NUM: u16 = 0xF4;

        let debug_exit_port = IoPort::acquire(DEBUG_EXIT_PORT_NUM).unwrap();

        DEBUG_EXIT_PORT.call_once(|| debug_exit_port);
        inject_poweroff_handler(try_exit_qemu);
    }
}

pub(super) fn init() {
    use super::cpu::cpuid;

    if !cpuid::query_is_running_in_qemu() {
        return;
    }

    // FIXME: We assume that the kernel is running in QEMU with the following QEMU command line
    // arguments that specify the isa-debug-exit device:
    // `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
    log::info!("QEMU hypervisor detected, assuming that the isa-debug-exit device exists");

    qemu_isa_debug::init();
}
