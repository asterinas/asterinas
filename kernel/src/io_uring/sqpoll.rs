// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::{cpu::CpuSet, sync::WaitQueue};

use super::{io_context::IoUringContext, thread::IoUringThreadOptions};
use crate::{prelude::*, thread::Thread};

const SQPOLL_IDLE_INTERVAL: Duration = Duration::from_millis(1);

pub(super) struct SqPoll {
    ring_context: Weak<IoUringContext>,
    state: Mutex<SqPollState>,
    wait_queue: WaitQueue,
}

struct SqPollState {
    is_sleeping: bool,
    should_stop: bool,
}

impl SqPoll {
    pub(super) fn new(ring_context: Weak<IoUringContext>) -> Self {
        Self {
            ring_context,
            state: Mutex::new(SqPollState {
                is_sleeping: false,
                should_stop: false,
            }),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn start_thread(
        self: &Arc<Self>,
        ctx: &Context,
        cpu_affinity: CpuSet,
        idle_timeout: Duration,
    ) {
        let sqpoll_thread_local =
            IoUringThreadOptions::clone_thread_local(ctx.thread_local).unwrap();

        let sqpoll = self.clone();
        IoUringThreadOptions::new(
            move || sqpoll_loop(sqpoll, idle_timeout),
            sqpoll_thread_local,
        )
        .cpu_affinity(cpu_affinity)
        .spawn();
    }

    pub(super) fn wake(&self) -> bool {
        let should_wake = {
            let mut state = self.state.lock();
            if !state.is_sleeping {
                false
            } else {
                state.is_sleeping = false;
                true
            }
        };

        if should_wake {
            self.wait_queue.wake_one();
        }
        should_wake
    }

    pub(super) fn stop(&self) {
        {
            let mut state = self.state.lock();
            state.should_stop = true;
            state.is_sleeping = false;
        }
        self.wait_queue.wake_all();
    }

    fn prepare_sleep(&self, context: &IoUringContext) -> Result<bool> {
        let mut state = self.state.lock();
        if state.should_stop {
            return Ok(false);
        }

        state.is_sleeping = true;

        if let Err(err) = context.set_sq_need_wakeup(true) {
            self.wake();
            return Err(err);
        }

        Ok(true)
    }

    fn wait_for_wakeup(&self) {
        self.wait_queue.wait_until(|| {
            let state = self.state.lock();
            (!state.is_sleeping || state.should_stop).then_some(())
        });
    }
}

fn sqpoll_loop(sqpoll: Arc<SqPoll>, idle_timeout: Duration) {
    let wait_queue = WaitQueue::new();
    let mut idle_elapsed = Duration::ZERO;

    loop {
        let Some(context) = sqpoll.ring_context.upgrade() else {
            break;
        };

        let submitted = match context.submit_sqes(u32::MAX) {
            Ok(submitted) => submitted,
            Err(err) => {
                warn!("failed to submit SQPOLL io_uring requests: {:?}", err);
                0
            }
        };

        if submitted == 0 {
            if idle_elapsed >= idle_timeout {
                let should_sleep = match sqpoll.prepare_sleep(&context) {
                    Ok(should_sleep) => should_sleep,
                    Err(err) => {
                        warn!("failed to put SQPOLL io_uring thread to sleep: {:?}", err);
                        false
                    }
                };
                drop(context);

                if should_sleep {
                    sqpoll.wait_for_wakeup();
                }
                idle_elapsed = Duration::ZERO;
                continue;
            }

            drop(context);
            let wait_interval = SQPOLL_IDLE_INTERVAL.min(idle_timeout - idle_elapsed);
            let _ = wait_queue.wait_until_or_timeout(|| -> Option<()> { None }, &wait_interval);
            idle_elapsed = idle_elapsed.saturating_add(wait_interval);
        } else {
            drop(context);
            idle_elapsed = Duration::ZERO;
            Thread::yield_now();
        }
    }
}
