use crate::task::{
    context_switch, get_idle_task_cx_ptr, Task, TaskContext, SWITCH_TO_USER_SPACE_TASK,
};

use super::{irq::IRQ_LIST, *};

#[no_mangle]
pub extern "C" fn syscall_handler(f: &'static mut SyscallFrame) -> isize {
    let r = &f.caller;
    let current = Task::current();
    current.inner_exclusive_access().is_from_trap = false;
    *current.syscall_frame() = *SWITCH_TO_USER_SPACE_TASK.get().syscall_frame();
    unsafe {
        context_switch(
            get_idle_task_cx_ptr() as *mut TaskContext,
            &Task::current().inner_ctx() as *const TaskContext,
        )
    }
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
    if !is_from_kernel(f.cs){
        let current = Task::current();
        current.inner_exclusive_access().is_from_trap = true;
    }
    let irq_line = IRQ_LIST.get(f.id as usize).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(f.clone());
    }
}


fn is_from_kernel(cs:usize)->bool{
    if cs&0x3==0{
        true
    }else{
        false
    }
}
