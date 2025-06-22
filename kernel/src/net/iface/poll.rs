// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::time::Duration;

use log::trace;
use ostd::timer::Jiffies;

use super::{iter_all_ifaces, Iface};
use crate::{
    sched::{Nice, SchedPolicy},
    thread::kernel_thread::ThreadOptions,
    WaitTimeout,
};

pub fn lazy_init() {
    for iface in iter_all_ifaces() {
        spawn_background_poll_thread(iface.clone());
    }
}

pub(super) fn poll_ifaces() {
    for iface in iter_all_ifaces() {
        iface.poll();
    }
}

fn spawn_background_poll_thread(iface: Arc<Iface>) {
    let task_fn = move || {
        trace!("spawn background poll thread for {}", iface.name());

        let sched_poll = iface.sched_poll();
        let wait_queue = sched_poll.polling_wait_queue();

        loop {
            let next_poll_at_ms = if let Some(next_poll_at_ms) = sched_poll.next_poll_at_ms() {
                next_poll_at_ms
            } else {
                wait_queue.wait_until(|| sched_poll.next_poll_at_ms())
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
            let _ = wait_queue.wait_until_or_timeout(
                // If `sched_poll.next_poll_at_ms()` changes to an earlier time, we will end the
                // waiting.
                || (sched_poll.next_poll_at_ms()? < next_poll_at_ms).then_some(()),
                &duration,
            );
        }
    };

    ThreadOptions::new(task_fn)
        .sched_policy(SchedPolicy::Fair(Nice::MIN))
        .spawn();
}
