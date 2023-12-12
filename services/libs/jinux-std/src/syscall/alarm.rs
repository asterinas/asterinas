use core::time::Duration;

use super::{SyscallReturn, SYS_ALARM};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::posix_thread::{PosixThreadExt, Timer};
use crate::process::signal::constants::SIGALRM;
use crate::process::signal::signals::kernel::KernelSignal;
use crate::thread::thread_table;

pub fn sys_alarm(seconds: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ALARM);
    debug!("seconds = {}", seconds);

    let current_thread = current_thread!();
    let mut timers = {
        let posix_thread = current_thread.as_posix_thread().unwrap();
        posix_thread.timers().lock()
    };

    timers.retain(|timer| !timer.is_expired() && !timer.is_cancelled());

    if seconds == 0 {
        // cancel all alarms
        timers.drain().for_each(|timer| {
            timer.cancel();
        });

        return Ok(SyscallReturn::Return(0));
    }

    let timer = {
        let timeout = Duration::from_secs(seconds as u64);
        Timer::new(
            |current_tid| {
                let signal = KernelSignal::new(SIGALRM);

                let process = {
                    let Some(current_thread) = thread_table::get_thread(current_tid) else {
                        return;
                    };
                    let posix_thread = current_thread.as_posix_thread().unwrap();
                    posix_thread.process()
                };

                process.enqueue_signal(signal);
            },
            timeout,
        )?
    };

    timers.push(timer);

    let min_remaining_secs = {
        let peek_timer = timers.peek().unwrap();
        let remaining = peek_timer.remain();
        if remaining.subsec_nanos() > 0 {
            remaining.as_secs() + 1
        } else {
            remaining.as_secs()
        }
    };

    Ok(SyscallReturn::Return(min_remaining_secs as _))
}
