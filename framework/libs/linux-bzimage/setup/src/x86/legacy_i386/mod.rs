// SPDX-License-Identifier: MPL-2.0

use linux_boot_params::BootParams;

use core::arch::{asm, global_asm};

global_asm!(include_str!("header.S"));

global_asm!(include_str!("setup.S"));

use crate::console::{print_hex, print_str};

pub const ASTER_ENTRY_POINT: u32 = 0x8001000;

#[export_name = "_bzimage_entry_32"]
extern "cdecl" fn bzimage_entry(boot_params_ptr: u32) -> ! {
    // Safety: this init function is only called once.
    unsafe { crate::console::init() };

    // println!("[setup] bzImage loaded at {:#x}", x86::relocation::get_image_loaded_offset());
    unsafe {
        print_str("[setup] bzImage loaded offset: ");
        print_hex(crate::x86::get_image_loaded_offset() as u64);
        print_str("\n");
    }

    // Safety: the boot_params_ptr is a valid pointer to be borrowed.
    let boot_params = unsafe { &*(boot_params_ptr as *const BootParams) };
    // Safety: the payload_offset and payload_length is valid.
    let payload = crate::get_payload(boot_params);
    crate::loader::load_elf(payload);

    // Safety: the entrypoint and the ptr is valid.
    unsafe { call_aster_entrypoint(ASTER_ENTRY_POINT, boot_params_ptr.try_into().unwrap()) };
}

unsafe fn call_aster_entrypoint(entrypoint: u32, boot_params_ptr: u32) -> ! {
    asm!("mov esi, {}", in(reg) boot_params_ptr);
    asm!("mov eax, {}", in(reg) entrypoint);
    asm!("jmp eax");

    unreachable!();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
