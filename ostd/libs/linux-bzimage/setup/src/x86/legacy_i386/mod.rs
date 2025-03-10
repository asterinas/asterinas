// SPDX-License-Identifier: MPL-2.0

use core::arch::{asm, global_asm};

use linux_boot_params::BootParams;

global_asm!(include_str!("setup.S"));

const ASTER_ENTRY_POINT: *const () = 0x8001000 as _;

#[export_name = "main_legacy32"]
extern "cdecl" fn main_legacy32(boot_params_ptr: *mut BootParams) -> ! {
    crate::println!(
        "[setup] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    crate::println!("[setup] Loading the payload as an ELF file");
    crate::loader::load_elf(crate::x86::payload());

    crate::println!(
        "[setup] Entering the Asterinas entry point at {:p}",
        ASTER_ENTRY_POINT,
    );
    // SAFETY:
    // 1. The entry point address is correct and matches the kernel ELF file.
    // 2. The boot parameter pointer is valid and points to the correct boot parameters.
    unsafe { call_aster_entrypoint(ASTER_ENTRY_POINT, boot_params_ptr) };
}

unsafe fn call_aster_entrypoint(entrypoint: *const (), boot_params_ptr: *mut BootParams) -> ! {
    unsafe {
        asm!(
            "mov esi, {1}",
            "mov eax, {0}",
            "jmp eax",
            in(reg) entrypoint,
            in(reg) boot_params_ptr,
            options(noreturn),
        )
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    crate::println!("[PANIC]: {}", info);

    loop {
        // SAFETY: `hlt` has no effect other than to stop the CPU and wait for another interrupt.
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}
