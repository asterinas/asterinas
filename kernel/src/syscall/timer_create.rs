// SPDX-License-Identifier: MPL-2.0

use super::{
    clock_gettime::{DynamicClockIdInfo, DynamicClockType},
    SyscallReturn,
};
use crate::{
    prelude::*,
    process::{
        posix_thread::{thread_table, AsPosixThread},
        process_table,
        signal::{
            c_types::{sigevent_t, SigNotify},
            constants::SIGALRM,
            sig_num::SigNum,
            signals::kernel::KernelSignal,
        },
    },
    syscall::ClockId,
    thread::work_queue::{submit_work_item, work_item::WorkItem},
    time::{
        clockid_t,
        clocks::{BootTimeClock, MonotonicClock, RealTimeClock},
    },
};

pub fn sys_timer_create(
    clockid: clockid_t,
    sigevent_addr: Vaddr,
    timer_id_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if timer_id_addr == 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "the address of timer_id_addr should be valid"
        );
    }

    let current_process = current!();
    let sent_signal: Box<dyn Fn() + Send + Sync + 'static> = {
        // If `sigevent_addr` is NULL, use the default method (like `sys_alarm`) to send signal.
        if sigevent_addr == 0 {
            let process = current_process.clone();
            let signal = KernelSignal::new(SIGALRM);
            Box::new(move || {
                process.enqueue_signal(signal);
            })
        // Determine the timeout action through `sigevent`.
        } else {
            let sig_event = ctx.user_space().read_val::<sigevent_t>(sigevent_addr)?;
            let sigev_notify = SigNotify::try_from(sig_event.sigev_notify)?;
            let signo = sig_event.sigev_signo;
            match sigev_notify {
                // Do nothing when the timer is expired.
                SigNotify::SIGEV_NONE => Box::new(|| {}),
                // Send a signal to the current process when the timer is expired.
                SigNotify::SIGEV_SIGNAL => {
                    let process = current_process.clone();
                    let signal = KernelSignal::new(SigNum::try_from(signo as u8)?);
                    Box::new(move || {
                        process.enqueue_signal(signal);
                    })
                }
                // Spawn a posix thread to run the `sigev_function`, which is stored in
                // `sig_event.sigev_un._sigev_thread`.
                //
                // TODO: enable this instructions. Currently the system does not provide an API to spawn
                // a posix thread to run a specified function.
                SigNotify::SIGEV_THREAD => {
                    unimplemented!()
                }
                // Send a signal to the specified thread when the timer is expired.
                SigNotify::SIGEV_THREAD_ID => {
                    let tid = sig_event.sigev_un.read_tid() as u32;
                    let thread = thread_table::get_thread(tid).ok_or_else(|| {
                        Error::with_message(Errno::EINVAL, "target thread does not exist")
                    })?;
                    let posix_thread = thread.as_posix_thread().unwrap();
                    if posix_thread.process().pid() != current_process.pid() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "target thread should belong to current process"
                        );
                    }
                    let signal = KernelSignal::new(SigNum::try_from(signo as u8)?);
                    Box::new(move || {
                        if let Some(thread) = thread.as_posix_thread() {
                            thread.enqueue_signal(Box::new(signal));
                        }
                    })
                }
            }
        }
    };

    let work_func = sent_signal;
    let work_item = WorkItem::new(work_func);
    let func = move || {
        submit_work_item(
            work_item.clone(),
            crate::thread::work_queue::WorkPriority::High,
        );
    };

    let process_timer_manager = current_process.timer_manager();
    let timer = if clockid >= 0 {
        let clock_id = ClockId::try_from(clockid)?;
        match clock_id {
            ClockId::CLOCK_PROCESS_CPUTIME_ID => process_timer_manager.create_prof_timer(func),
            ClockId::CLOCK_THREAD_CPUTIME_ID => ctx.posix_thread.create_prof_timer(func),
            ClockId::CLOCK_REALTIME => RealTimeClock::timer_manager().create_timer(func),
            ClockId::CLOCK_MONOTONIC => MonotonicClock::timer_manager().create_timer(func),
            ClockId::CLOCK_BOOTTIME => BootTimeClock::timer_manager().create_timer(func),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid clock ID"),
        }
    } else {
        let dynamic_clockid_info = DynamicClockIdInfo::try_from(clockid)?;
        match dynamic_clockid_info {
            DynamicClockIdInfo::Pid(pid, clock_type) => {
                let process = process_table::get_process(pid)
                    .ok_or_else(|| crate::Error::with_message(Errno::EINVAL, "invalid clock id"))?;
                let process_timer_manager = process.timer_manager();
                match clock_type {
                    DynamicClockType::Profiling => process_timer_manager.create_prof_timer(func),
                    DynamicClockType::Virtual => process_timer_manager.create_virtual_timer(func),
                    // TODO: support scheduling clock and fd clock.
                    _ => unimplemented!(),
                }
            }
            DynamicClockIdInfo::Tid(tid, clock_type) => {
                let thread = thread_table::get_thread(tid)
                    .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid clock id"))?;
                let posix_thread = thread.as_posix_thread().unwrap();
                match clock_type {
                    DynamicClockType::Profiling => posix_thread.create_prof_timer(func),
                    DynamicClockType::Virtual => posix_thread.create_virtual_timer(func),
                    _ => unimplemented!(),
                }
            }
            DynamicClockIdInfo::Fd(_) => unimplemented!(),
        }
    };

    let timer_id = process_timer_manager.add_posix_timer(timer);
    ctx.user_space().write_val(timer_id_addr, &timer_id)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_timer_delete(timer_id: usize, _ctx: &Context) -> Result<SyscallReturn> {
    let current_process = current!();
    let Some(timer) = current_process.timer_manager().remove_posix_timer(timer_id) else {
        return_errno_with_message!(Errno::EINVAL, "invalid timer ID");
    };

    timer.cancel();
    Ok(SyscallReturn::Return(0))
}
