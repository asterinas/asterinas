// SPDX-License-Identifier: MPL-2.0

use aster_frame::timer::read_monotonic_milli_seconds;

use crate::{
    prelude::*,
    thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    },
};

use super::Iface;

pub enum BindPortConfig {
    CanReuse(u16),
    Specified(u16),
    Ephemeral,
}

impl BindPortConfig {
    pub fn new(port: u16, can_reuse: bool) -> Result<Self> {
        let config = if port != 0 {
            if can_reuse {
                Self::CanReuse(port)
            } else {
                Self::Specified(port)
            }
        } else if can_reuse {
            return_errno_with_message!(Errno::EINVAL, "invalid bind port config");
        } else {
            Self::Ephemeral
        };
        Ok(config)
    }

    pub(super) fn can_reuse(&self) -> bool {
        matches!(self, Self::CanReuse(_))
    }

    pub(super) fn port(&self) -> Option<u16> {
        match self {
            Self::CanReuse(port) | Self::Specified(port) => Some(*port),
            Self::Ephemeral => None,
        }
    }
}

pub fn spawn_background_poll_thread(iface: Arc<dyn Iface>) {
    // FIXME: use timer or wait_timeout when timer is enable.
    let task_fn = move || {
        debug!("spawn background poll thread");
        loop {
            let next_poll_time = if let Some(next_poll_time) = iface.next_poll_at_ms() {
                next_poll_time
            } else {
                Thread::yield_now();
                continue;
            };
            let now = read_monotonic_milli_seconds();
            if now > next_poll_time {
                // FIXME: now is later than next poll time. This may cause problem.
                iface.poll();
                continue;
            }
            let duration = next_poll_time - now;
            // FIXME: choose a suitable time interval
            if duration < 10 {
                iface.poll();
            } else {
                Thread::yield_now();
            }
        }
    };
    Thread::spawn_kernel_thread(ThreadOptions::new(task_fn));
}
