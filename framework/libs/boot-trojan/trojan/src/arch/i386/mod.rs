use crate::println;

use core::arch::{asm, global_asm};

global_asm!(include_str!("header.S"));

global_asm!(include_str!("setup.S"));

#[no_mangle]
extern "cdecl" fn _trojan_entry_32(boot_params_ptr: u32) -> ! {
    crate::trojan_entry(0x100000, boot_params_ptr.try_into().unwrap());
}

pub const ASTER_ENTRY_POINT: u32 = 0x8001000;

pub unsafe fn call_aster_entrypoint(entrypoint: u32, boot_params_ptr: u32) -> ! {
    asm!("mov esi, {}", in(reg) boot_params_ptr);
    asm!("mov eax, {}", in(reg) entrypoint);
    asm!("jmp eax");

    unreachable!();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {:?}", info);
    loop {}
}
