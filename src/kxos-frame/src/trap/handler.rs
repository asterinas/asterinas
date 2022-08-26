use super::{irq::IRQ_LIST, *};

#[no_mangle]
pub extern "C" fn syscall_handler(f: &'static mut SyscallFrame) -> isize {
    let r = &f.caller;
    println!("{:?}", f);
    // let ret = syscall::syscall(r.rax, [r.rdi, r.rsi, r.rdx]);
    // current_check_signal();
    // ret
    -1
}

const DIVIDE_BY_ZERO: usize = 0;
const INVALID_OPCODE: usize = 6;
const SEGMENT_NOT_PRESENT: usize = 11;
const STACK_SEGMENT_FAULT: usize = 12;
const GENERAL_PROTECTION_FAULT: usize = 13;
const PAGE_FAULT: usize = 14;
const TIMER: usize = 32;

#[no_mangle]
pub extern "C" fn trap_handler(f: &'static mut TrapFrame) {
    let irq_line = IRQ_LIST.get(f.id as usize).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(f.clone());
    }
}
