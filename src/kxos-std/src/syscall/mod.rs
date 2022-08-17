use alloc::vec;
use alloc::{sync::Arc, vec::Vec};
use kxos_frame::Error;
use kxos_frame::cpu::CpuContext;
use kxos_frame::{task::Task, user::UserSpace, vm::VmIo};

use kxos_frame::info;

const SYS_WRITE: u64 = 64;
const SYS_EXIT: u64 = 93;

pub struct SyscallFrame {
    syscall_number: u64,
    args: [u64; 6],
}

impl SyscallFrame {
    fn new_from_context(context: &CpuContext) -> Self {
        let syscall_number = context.gp_regs.rax;
        let mut args = [0u64; 6];
        args[0] = context.gp_regs.rdi;
        args[1] = context.gp_regs.rsi;
        args[2] = context.gp_regs.rdx;
        args[3] = context.gp_regs.r10;
        args[4] = context.gp_regs.r8;
        args[5] = context.gp_regs.r9;
        Self {
            syscall_number, args,
        }
    }
}

pub fn syscall_handler(context: &mut CpuContext) {
    let syscall_frame = SyscallFrame::new_from_context(context);
    let syscall_return = syscall_dispatch(syscall_frame.syscall_number, syscall_frame.args);

    // FIXME: set return value?
    context.gp_regs.rax = syscall_return as u64;
}

pub fn syscall_dispatch(syscall_number: u64, args: [u64; 6]) -> isize {
    match syscall_number {
        SYS_WRITE => sys_write(args[0], args[1], args[2]),
        SYS_EXIT => sys_exit(args[0] as _),
        _ => panic!("Unsupported syscall number: {}", syscall_number),
    }
}

pub fn sys_write(fd: u64, user_buf_ptr: u64, user_buf_len: u64) -> isize {
    // only suppprt STDOUT now.
    const STDOUT: u64 = 1;
    if fd == STDOUT {
        let task = Task::current();
        let user_space = task.user_space().expect("No user space attached");
        let user_buffer = copy_bytes_from_user(user_space, user_buf_ptr as usize, user_buf_len as usize)
            .expect("read user buffer failed");
        let content = alloc::str::from_utf8(user_buffer.as_slice()).expect("Invalid content");
        // TODO: print content
        info!("Message from user mode: {}", content);
        0
    } else {
        panic!("Unsupported fd number {}", fd);
    }
}

pub fn sys_exit(exit_code: i32) -> isize {
    // let current = Task::current();
    // current.exit(exit_code);
    todo!()
}

fn copy_bytes_from_user(
    user_space: &Arc<UserSpace>,
    user_buf_ptr: usize,
    user_buf_len: usize,
) -> Result<Vec<u8>, Error> {
    let vm_space = user_space.vm_space();
    let mut buffer = vec![0u8; user_buf_len];
    vm_space.read_bytes(user_buf_ptr, &mut buffer)?;
    Ok(buffer)
}
