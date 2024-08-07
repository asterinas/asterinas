// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use crate::{
    prelude::*,
    process::posix_thread::futex::{
        futex_op_and_flags_from_u32, futex_requeue, futex_wait, futex_wait_bitset, futex_wake,
        futex_wake_bitset, FutexFlags, FutexOp,
    },
    syscall::SyscallReturn,
    time::{
        clocks::{MonotonicClock, RealTimeClock},
        timer::Timeout,
        timespec_t,
        wait::WakerTimerCreater,
    },
    util::read_val_from_user,
};

pub fn sys_futex(
    futex_addr: Vaddr,
    futex_op: i32,
    futex_val: u64,
    utime_addr: Vaddr,
    futex_new_addr: u64,
    bitset: u64,
) -> Result<SyscallReturn> {
    // FIXME: we current ignore futex flags
    let (futex_op, futex_flags) = futex_op_and_flags_from_u32(futex_op as _).unwrap();
    debug!(
        "futex_op = {:?}, futex_flags = {:?}, futex_addr = 0x{:x}, futex_val = 0x{:x}",
        futex_op, futex_flags, futex_addr, futex_val
    );

    let get_futex_val = |val: i32| -> Result<usize> {
        if val < 0 {
            return_errno_with_message!(Errno::EINVAL, "the futex val must not be negative");
        }
        Ok(val as usize)
    };

    let get_futex_timer_creater =
        |timeout_addr: Vaddr| -> Result<Option<WakerTimerCreater<'static>>> {
            if timeout_addr == 0 {
                return Ok(None);
            }

            let timeout = {
                let time_spec: timespec_t = read_val_from_user(timeout_addr)?;
                Duration::from(time_spec)
            };

            let is_real_time = futex_flags.contains(FutexFlags::FUTEX_CLOCK_REALTIME);
            if is_real_time && futex_op == FutexOp::FUTEX_WAIT {
                // Ref: <https://github.com/torvalds/linux/commit/4fbf5d6837bf81fd7a27d771358f4ee6c4f243f8>
                return_errno_with_message!(Errno::ENOSYS, "FUTEX_WAIT cannot use CLOCK_REALTIME");
            }

            let timeout = {
                // From man(2) futex:
                // for FUTEX_WAIT, timeout is interpreted as a relative value.
                // This differs from other futex operations,
                // where timeout is interpreted as an absolute value.
                // To obtain the equivalent of FUTEX_WAIT with an absolute timeout,
                // employ FUTEX_WAIT_BITSET with val3 specified as FUTEX_BITSET_MATCH_ANY.
                if futex_op == FutexOp::FUTEX_WAIT {
                    Timeout::After(timeout)
                } else {
                    Timeout::When(timeout)
                }
            };

            let timer_manager = if is_real_time {
                debug!("futex timeout = {:?}, clock = CLOCK_REALTIME", timeout);
                RealTimeClock::timer_manager()
            } else {
                debug!("futex timeout = {:?}, clock = CLOCK_MONOTONIC", timeout);
                MonotonicClock::timer_manager()
            };

            Ok(Some(WakerTimerCreater::new_with_timer_manager(
                timeout,
                timer_manager,
            )))
        };

    let res = match futex_op {
        FutexOp::FUTEX_WAIT => {
            let timer_creater = get_futex_timer_creater(utime_addr)?;
            futex_wait(futex_addr as _, futex_val as _, timer_creater).map(|_| 0)
        }
        FutexOp::FUTEX_WAIT_BITSET => {
            let timeout = get_futex_timer_creater(utime_addr)?;
            futex_wait_bitset(futex_addr as _, futex_val as _, timeout, bitset as _).map(|_| 0)
        }
        FutexOp::FUTEX_WAKE => {
            let max_count = get_futex_val(futex_val as i32)?;
            futex_wake(futex_addr as _, max_count).map(|count| count as isize)
        }
        FutexOp::FUTEX_WAKE_BITSET => {
            let max_count = get_futex_val(futex_val as i32)?;
            futex_wake_bitset(futex_addr as _, max_count, bitset as _).map(|count| count as isize)
        }
        FutexOp::FUTEX_REQUEUE => {
            let max_nwakes = get_futex_val(futex_val as i32)?;
            let max_nrequeues = get_futex_val(utime_addr as i32)?;
            futex_requeue(
                futex_addr as _,
                max_nwakes,
                max_nrequeues,
                futex_new_addr as _,
            )
            .map(|nwakes| nwakes as _)
        }
        _ => {
            warn!("futex op = {:?}", futex_op);
            return_errno_with_message!(Errno::EINVAL, "unsupported futex op");
        }
    }
    .map_err(|e| {
        // From Linux manual, Futex returns `ETIMEDOUT` instead of `ETIME`
        if e.error() == Errno::ETIME {
            Error::with_message(Errno::ETIMEDOUT, "futex wait timeout")
        } else {
            e
        }
    })?;

    debug!("futex returns, tid= {} ", current_thread!().tid());
    Ok(SyscallReturn::Return(res as _))
}
