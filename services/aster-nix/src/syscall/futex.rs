// SPDX-License-Identifier: MPL-2.0

use crate::process::posix_thread::futex::{
    futex_op_and_flags_from_u32, futex_requeue, futex_wait, futex_wait_bitset, futex_wake,
    futex_wake_bitset, FutexOp, FutexTimeout,
};
use crate::syscall::SyscallReturn;
use crate::syscall::SYS_FUTEX;

use crate::{log_syscall_entry, prelude::*};

pub fn sys_futex(
    futex_addr: Vaddr,
    futex_op: i32,
    futex_val: u32,
    utime_addr: u64,
    futex_new_addr: u64,
    bitset: u64,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FUTEX);
    // FIXME: we current ignore futex flags
    let (futex_op, futex_flags) = futex_op_and_flags_from_u32(futex_op as _).unwrap();
    debug!(
        "futex_op = {:?}, futex_flags = {:?}, futex_addr = 0x{:x}",
        futex_op, futex_flags, futex_addr
    );

    let get_futex_val = |val: i32| -> Result<usize> {
        if val < 0 {
            return_errno_with_message!(Errno::EINVAL, "the futex val must not be negative");
        }
        Ok(val as usize)
    };

    let get_futex_timeout = |timeout_addr| -> Result<Option<FutexTimeout>> {
        if timeout_addr == 0 {
            return Ok(None);
        }
        // TODO: parse a timeout
        todo!()
    };

    let res = match futex_op {
        FutexOp::FUTEX_WAIT => {
            let timeout = get_futex_timeout(utime_addr).expect("Invalid time addr");
            futex_wait(futex_addr as _, futex_val as _, &timeout).map(|_| 0)
        }
        FutexOp::FUTEX_WAIT_BITSET => {
            let timeout = get_futex_timeout(utime_addr).expect("Invalid time addr");
            futex_wait_bitset(futex_addr as _, futex_val as _, &timeout, bitset as _).map(|_| 0)
        }
        FutexOp::FUTEX_WAKE => {
            let max_count = get_futex_val(futex_val as i32).expect("Invalid futex val");
            futex_wake(futex_addr as _, max_count).map(|count| count as isize)
        }
        FutexOp::FUTEX_WAKE_BITSET => {
            let max_count = get_futex_val(futex_val as i32).expect("Invalid futex val");
            futex_wake_bitset(futex_addr as _, max_count, bitset as _).map(|count| count as isize)
        }
        FutexOp::FUTEX_REQUEUE => {
            let max_nwakes = get_futex_val(futex_val as i32).expect("Invalid futex val");
            let max_nrequeues = get_futex_val(utime_addr as i32).expect("Invalid utime addr");
            futex_requeue(
                futex_addr as _,
                max_nwakes,
                max_nrequeues,
                futex_new_addr as _,
            )
            .map(|nwakes| nwakes as _)
        }
        _ => panic!("Unsupported futex operations"),
    }
    .unwrap();

    debug!("futex returns, tid= {} ", current_thread!().tid());
    Ok(SyscallReturn::Return(res as _))
}
