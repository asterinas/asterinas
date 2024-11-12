// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::time::Duration;

use id_alloc::IdAlloc;
use ostd::{
    arch::{timer::TIMER_FREQ, trap::is_kernel_interrupted},
    sync::Mutex,
    timer,
};

use super::Process;
use crate::{
    process::{
        posix_thread::AsPosixThread,
        signal::{constants::SIGALRM, signals::kernel::KernelSignal},
    },
    thread::{
        work_queue::{submit_work_item, work_item::WorkItem},
        Thread,
    },
    time::{
        clocks::{ProfClock, RealTimeClock},
        Timer, TimerManager,
    },
};

/// Updates the CPU time recorded in the CPU clocks of current Process.
///
/// This function will be invoked at the system timer interrupt, and
/// invoke the callbacks of expired timers which are based on the updated
/// CPU clock.
fn update_cpu_time() {
    let Some(current_thread) = Thread::current() else {
        return;
    };
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return;
    };
    let process = posix_thread.process();
    let timer_manager = process.timer_manager();
    let jiffies_interval = Duration::from_millis(1000 / TIMER_FREQ);
    // Based on whether the timer interrupt occurs in kernel mode or user mode,
    // the function will add the duration of one timer interrupt interval to the
    // corresponding CPU clocks.
    if is_kernel_interrupted() {
        posix_thread
            .prof_clock()
            .kernel_clock()
            .add_time(jiffies_interval);
        process
            .prof_clock()
            .kernel_clock()
            .add_time(jiffies_interval);
    } else {
        posix_thread
            .prof_clock()
            .user_clock()
            .add_time(jiffies_interval);
        process.prof_clock().user_clock().add_time(jiffies_interval);
        timer_manager
            .virtual_timer()
            .timer_manager()
            .process_expired_timers();
    }
    timer_manager
        .prof_timer()
        .timer_manager()
        .process_expired_timers();
    posix_thread.process_expired_timers();
}

/// Registers a function to update the CPU clock in processes and
/// threads during the system timer interrupt.
pub(super) fn init() {
    timer::register_callback(update_cpu_time);
}

/// Represents timer resources and utilities for a POSIX process.
pub struct PosixTimerManager {
    /// A real-time countdown timer, measuring in wall clock time.
    alarm_timer: Arc<Timer>,
    /// A timer based on user CPU clock.
    virtual_timer: Arc<Timer>,
    /// A timer based on the profiling clock.
    prof_timer: Arc<Timer>,
    /// An ID allocator to allocate unique timer IDs.
    id_allocator: Mutex<IdAlloc>,
    /// A container managing all POSIX timers created by `timer_create()` syscall
    /// within the process context.
    posix_timers: Mutex<Vec<Option<Arc<Timer>>>>,
}

fn create_process_timer_callback(process_ref: &Weak<Process>) -> impl Fn() + Clone {
    let current_process = process_ref.clone();
    let sent_signal = move || {
        let signal = KernelSignal::new(SIGALRM);
        if let Some(process) = current_process.upgrade() {
            process.enqueue_signal(signal);
        }
    };

    let work_func = Box::new(sent_signal);
    let work_item = WorkItem::new(work_func);

    move || {
        submit_work_item(
            work_item.clone(),
            crate::thread::work_queue::WorkPriority::High,
        );
    }
}

impl PosixTimerManager {
    pub(super) fn new(prof_clock: &Arc<ProfClock>, process_ref: &Weak<Process>) -> Self {
        const MAX_NUM_OF_POSIX_TIMERS: usize = 10000;

        let callback = create_process_timer_callback(process_ref);

        let alarm_timer = RealTimeClock::timer_manager().create_timer(callback.clone());

        let virtual_timer =
            TimerManager::new(prof_clock.user_clock().clone()).create_timer(callback.clone());
        let prof_timer = TimerManager::new(prof_clock.clone()).create_timer(callback);

        Self {
            alarm_timer,
            virtual_timer,
            prof_timer,
            id_allocator: Mutex::new(IdAlloc::with_capacity(MAX_NUM_OF_POSIX_TIMERS)),
            posix_timers: Mutex::new(Vec::new()),
        }
    }

    /// Gets the alarm timer of the corresponding process.
    pub fn alarm_timer(&self) -> &Arc<Timer> {
        &self.alarm_timer
    }

    /// Gets the virtual timer of the corresponding process.
    pub fn virtual_timer(&self) -> &Arc<Timer> {
        &self.virtual_timer
    }

    /// Gets the profiling timer of the corresponding process.
    pub fn prof_timer(&self) -> &Arc<Timer> {
        &self.prof_timer
    }

    /// Creates a timer based on the profiling CPU clock of the current process.
    pub fn create_prof_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.prof_timer.timer_manager().create_timer(func)
    }

    /// Creates a timer based on the user CPU clock of the current process.
    pub fn create_virtual_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.virtual_timer.timer_manager().create_timer(func)
    }

    /// Adds a POSIX timer to the managed `posix_timers`, and allocate a timer ID for this timer.
    /// Return the timer ID.
    pub fn add_posix_timer(&self, posix_timer: Arc<Timer>) -> usize {
        let mut timers = self.posix_timers.lock();
        // Holding the lock of `posix_timers` is required to operate the `id_allocator`.
        let timer_id = self.id_allocator.lock().alloc().unwrap();
        if timers.len() < timer_id + 1 {
            timers.resize(timer_id + 1, None);
        }
        // The ID allocated is not used by any other timers so this index in `timers`
        // must be `None`.
        timers[timer_id] = Some(posix_timer);
        timer_id
    }

    /// Finds a POSIX timer by the input `timer_id`.
    pub fn find_posix_timer(&self, timer_id: usize) -> Option<Arc<Timer>> {
        let timers = self.posix_timers.lock();
        if timer_id >= timers.len() {
            return None;
        }

        timers[timer_id].clone()
    }

    /// Removes the POSIX timer with the ID `timer_id`.
    pub fn remove_posix_timer(&self, timer_id: usize) -> Option<Arc<Timer>> {
        let mut timers = self.posix_timers.lock();
        if timer_id >= timers.len() {
            return None;
        }

        let timer = timers[timer_id].take();
        if timer.is_some() {
            // Holding the lock of `posix_timers` is required to operate the `id_allocator`.
            self.id_allocator.lock().free(timer_id);
        }
        timer
    }
}
