// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;
use spin::Once;

use super::{
    SyscallReturn,
    clock_gettime::{DynamicClockIdInfo, DynamicClockType},
};
use crate::{
    prelude::*,
    process::{
        Process, pid_table,
        posix_thread::AsPosixThread,
        signal::{
            c_types::{SigNotify, sigevent_t},
            constants::SIGALRM,
            sig_num::SigNum,
            signals::timer::TimerSignal,
        },
    },
    syscall::ClockId,
    thread::{
        Thread,
        work_queue::{WorkPriority, submit_work_item, work_item::WorkItem},
    },
    time::{
        Timer, clockid_t,
        clocks::{BootTimeClock, MonotonicClock, RealTimeClock},
        timer::{TimerFn, TimerGuard},
        timer_t,
    },
};

pub(super) fn sys_timer_create(
    clockid: clockid_t,
    sigevent_addr: Vaddr,
    timer_id_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let signal_meta = if sigevent_addr == 0 {
        // If `sigevent_addr` is NULL, send SIGALRM.
        Some(SignalMeta {
            target: SignalTarget::Process(Arc::downgrade(&ctx.process)),
            num: SIGALRM,
            ptr: None,
        })
    } else {
        // Otherwise, determine the signal through `sigevent`.
        let sig_event = ctx.user_space().read_val::<sigevent_t>(sigevent_addr)?;
        let sigev_notify = SigNotify::try_from(sig_event.sigev_notify)?;
        let signo = sig_event.sigev_signo;
        match sigev_notify {
            // Do nothing when the timer is expired.
            SigNotify::SIGEV_NONE => None,
            // Send a signal to the current process when the timer is expired.
            //
            // `SIGEV_THREAD` means to spawn a POSIX thread to run `sigev_function` (i.e.,
            // `sig_event.sigev_un.sigev_thread`). However, this is handled by the user space
            // library (e.g., glibc). The kernel treats it the same as `SIGEV_SIGNAL`.
            SigNotify::SIGEV_SIGNAL | SigNotify::SIGEV_THREAD => {
                let signum = u8::try_from(signo)
                    .ok()
                    .and_then(|signo| SigNum::try_from(signo).ok())
                    .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid signal number"))?;
                Some(SignalMeta {
                    target: SignalTarget::Process(Arc::downgrade(&ctx.process)),
                    num: signum,
                    ptr: Some(sig_event.sigev_value.read_ptr()),
                })
            }
            // Send a signal to the specified thread when the timer is expired.
            SigNotify::SIGEV_THREAD_ID => {
                let tid = sig_event.sigev_un.read_tid() as u32;
                let thread = pid_table::pid_table_mut().get_thread(tid).ok_or_else(|| {
                    Error::with_message(Errno::EINVAL, "the target thread does not exist")
                })?;
                let posix_thread = thread.as_posix_thread().unwrap();
                if posix_thread.weak_process().as_ptr() != ctx.process.as_ref() {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the target thread does not belong to the current process"
                    );
                }
                let signum = u8::try_from(signo)
                    .ok()
                    .and_then(|signo| SigNum::try_from(signo).ok())
                    .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid signal number"))?;
                Some(SignalMeta {
                    target: SignalTarget::Thread(Arc::downgrade(&thread)),
                    num: signum,
                    ptr: Some(sig_event.sigev_value.read_ptr()),
                })
            }
        }
    };

    let signal_fn = SignalFn {
        work_item: Once::new(),
    };
    let timer = create_timer(clockid, signal_fn, ctx)?;

    let Some(timer_id) = ctx.process.timer_manager().add_posix_timer(timer.clone()) else {
        return_errno_with_message!(Errno::EAGAIN, "timer IDs are exhausted");
    };
    if let Err(error) = ctx.user_space().write_val(timer_id_addr, &timer_id) {
        let _ = ctx.process.timer_manager().remove_posix_timer(timer_id);
        return Err(error.into());
    }

    // The partially initialized timer is visible to the user for a brief moment. However, user
    // programs should not use this timer until the syscall has returned.
    if let Some(meta) = signal_meta {
        let work_item = meta.into_work_item(timer_id);
        timer.func().work_item.call_once(|| work_item);
    }

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_timer_delete(timer_id: timer_t, ctx: &Context) -> Result<SyscallReturn> {
    let Some(timer) = ctx.process.timer_manager().remove_posix_timer(timer_id) else {
        return_errno_with_message!(Errno::EINVAL, "invalid timer ID");
    };

    timer.lock().cancel();
    Ok(SyscallReturn::Return(0))
}

/// Creates a timer associated with the specified fixed or dynamic clock ID.
///
/// This timer will invoke the given callback function (`func`) when it expires.
fn create_timer<F: TimerFn>(clockid: clockid_t, func: F, ctx: &Context) -> Result<Arc<Timer<F>>> {
    let timer = if clockid >= 0 {
        return create_timer_for_clock(ClockId::try_from(clockid)?, func, ctx);
    } else {
        let dynamic_clockid_info = DynamicClockIdInfo::try_from(clockid)?;
        match dynamic_clockid_info {
            DynamicClockIdInfo::Pid(pid, clock_type) => {
                let process = pid_table::pid_table_mut()
                    .get_process(pid)
                    .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid clock ID"))?;
                let process_timer_manager = process.timer_manager();
                match clock_type {
                    DynamicClockType::Profiling => process_timer_manager.create_prof_timer(func),
                    DynamicClockType::Virtual => process_timer_manager.create_virtual_timer(func),
                }
            }
            DynamicClockIdInfo::Tid(tid, clock_type) => {
                let thread = pid_table::pid_table_mut()
                    .get_thread(tid)
                    .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid clock ID"))?;
                let posix_thread = thread.as_posix_thread().unwrap();
                match clock_type {
                    DynamicClockType::Profiling => posix_thread.create_prof_timer(func),
                    DynamicClockType::Virtual => posix_thread.create_virtual_timer(func),
                }
            }
        }
    };
    Ok(timer)
}

/// Creates a timer associated with the specified clock ID.
///
/// This timer will invoke the given callback function (`func`) when it expires.
pub(crate) fn create_timer_for_clock<F: TimerFn>(
    clock_id: ClockId,
    func: F,
    ctx: &Context,
) -> Result<Arc<Timer<F>>> {
    let process_timer_manager = ctx.process.timer_manager();
    let timer = match clock_id {
        ClockId::CLOCK_PROCESS_CPUTIME_ID => process_timer_manager.create_prof_timer(func),
        ClockId::CLOCK_THREAD_CPUTIME_ID => ctx.posix_thread.create_prof_timer(func),
        ClockId::CLOCK_REALTIME => RealTimeClock::timer_manager().create_timer(func),
        ClockId::CLOCK_MONOTONIC => MonotonicClock::timer_manager().create_timer(func),
        ClockId::CLOCK_BOOTTIME => BootTimeClock::timer_manager().create_timer(func),
        _ => return_errno_with_message!(Errno::EINVAL, "invalid clock ID"),
    };
    Ok(timer)
}

/// A function that delivers a signal via a [`WorkItem`].
struct SignalFn {
    work_item: Once<Arc<WorkItem>>,
}

impl TimerFn for SignalFn {
    fn call(&self, guard: TimerGuard) {
        drop(guard);

        let Some(work_item) = self.work_item.get() else {
            // The timer has not been initialized, or there is no action to perform (SIGEV_NONE).
            return;
        };
        submit_work_item(work_item.clone(), WorkPriority::High);
    }
}

/// Metadata to construct a [`TimerSignal`] and a [`WorkItem`] to deliver it.
struct SignalMeta {
    target: SignalTarget,
    num: SigNum,
    ptr: Option<Vaddr>,
}

enum SignalTarget {
    Thread(Weak<Thread>),
    Process(Weak<Process>),
}

impl SignalMeta {
    fn into_work_item(self, timer_id: i32) -> Arc<WorkItem> {
        // Use the timer ID as the default `sigev_value` value when the `sigevent` struct is absent.
        let signal = TimerSignal::new(self.num, timer_id, self.ptr.unwrap_or(timer_id as Vaddr));

        match self.target {
            SignalTarget::Thread(thread) => WorkItem::new(Box::new(move || {
                if let Some(thread) = thread.upgrade() {
                    thread
                        .as_posix_thread()
                        .unwrap()
                        .enqueue_signal(Box::new(signal));
                }
            })),
            SignalTarget::Process(process) => WorkItem::new(Box::new(move || {
                if let Some(process) = process.upgrade() {
                    process.enqueue_signal(Box::new(signal));
                }
            })),
        }
    }
}
