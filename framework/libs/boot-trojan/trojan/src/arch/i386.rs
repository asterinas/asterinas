use core::arch::{asm, global_asm};

global_asm!(include_str!("header.S"));

global_asm!(include_str!("setup.S"));

#[no_mangle]
extern "cdecl" fn _trojan_entry_32(boot_params_ptr: u32) -> ! {
    crate::trojan_entry(boot_params_ptr);
}

pub unsafe fn call_aster_entrypoint(entrypoint: u32, boot_params_ptr: u32) -> ! {
    asm!("mov esi, {}", in(reg) boot_params_ptr);
    asm!("mov eax, {}", in(reg) entrypoint);
    asm!("jmp eax");

    unreachable!();
}
