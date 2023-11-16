use core::arch::{asm, global_asm};

global_asm!(include_str!("header.S"));

global_asm!(include_str!("setup64.S"));

#[no_mangle]
extern "cdecl" fn _trojan_entry_64(boot_params_ptr: u64) -> ! {
    crate::trojan_entry(boot_params_ptr as u32);
}

pub unsafe fn call_aster_entrypoint(entrypoint: u64, boot_params_ptr: u64) -> ! {
    asm!("mov rsi, {}", in(reg) boot_params_ptr as u64);
    asm!("mov rax, {}", in(reg) entrypoint as u64);
    asm!("jmp rax");

    unreachable!();
}
