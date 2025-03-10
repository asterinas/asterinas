// SPDX-License-Identifier: MPL-2.0

use core::arch::{asm, global_asm};

global_asm!(include_str!("setup.S"));

pub const ASTER_ENTRY_POINT: u32 = 0x8001000;

#[export_name = "main_legacy32"]
extern "cdecl" fn main_legacy32(boot_params_ptr: u32) -> ! {
    crate::println!(
        "[setup] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    crate::println!("[setup] Loading the payload as an ELF file");
    crate::loader::load_elf(crate::x86::payload());

    crate::println!(
        "[setup] Entering the Asterinas entry point at {:#x}",
        ASTER_ENTRY_POINT,
    );
    // SAFETY: the entrypoint and the ptr is valid.
    unsafe { call_aster_entrypoint(ASTER_ENTRY_POINT, boot_params_ptr.try_into().unwrap()) };
}

unsafe fn call_aster_entrypoint(entrypoint: u32, boot_params_ptr: u32) -> ! {
    asm!("mov esi, {}", in(reg) boot_params_ptr);
    asm!("mov eax, {}", in(reg) entrypoint);
    asm!("jmp eax");

    unreachable!();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    crate::println!("[PANIC]: {}", info);

    loop {
        // SAFETY: `hlt` has no effect other than to stop the CPU and wait for another interrupt.
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}
