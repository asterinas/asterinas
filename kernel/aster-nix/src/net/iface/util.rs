// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::{arch::timer::Jiffies, task::Priority};

use super::Iface;
use crate::{
    prelude::*,
    thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    },
    time::wait::WaitTimeout,
};

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
    let task_fn = move || {
        trace!("spawn background poll thread for {}", iface.name());
        let wait_queue = iface.polling_wait_queue();
        loop {
            let next_poll_at_ms = if let Some(next_poll_at_ms) = iface.next_poll_at_ms() {
                next_poll_at_ms
            } else {
                wait_queue.wait_until(|| iface.next_poll_at_ms())
            };

            let now_as_ms = Jiffies::elapsed().as_duration().as_millis() as u64;

            // FIXME: Ideally, we should perform the `poll` just before `next_poll_at_ms`.
            // However, this approach may result in a spinning busy loop
            // if the `poll` operation yields no results.
            // To mitigate this issue,
            // we have opted to assign a high priority to the polling thread,
            // ensuring that the `poll` runs as soon as possible.
            // For a more in-depth discussion, please refer to the following link:
            // <https://github.com/asterinas/asterinas/pull/630#discussion_r1496817030>.
            if now_as_ms >= next_poll_at_ms {
                iface.poll();
                continue;
            }

            let duration = Duration::from_millis(next_poll_at_ms - now_as_ms);
            wait_queue.wait_until_or_timeout(
                // If `iface.next_poll_at_ms()` changes to an earlier time, we will end the waiting.
                || (iface.next_poll_at_ms()? < next_poll_at_ms).then_some(()),
                &duration,
            );
        }
    };

    let options = ThreadOptions::new(task_fn).priority(Priority::high());
    Thread::spawn_kernel_thread(options);
}
