// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::time::Duration;

use log::trace;
use ostd::timer::Jiffies;

use super::{ext::IfaceEx, Iface, IFACES};
use crate::{sched::priority::Priority, thread::kernel_thread::ThreadOptions, WaitTimeout};

pub fn lazy_init() {
    for iface in IFACES.get().unwrap() {
        spawn_background_poll_thread(iface.clone());
    }
}

pub(super) fn poll_ifaces() {
    let ifaces = IFACES.get().unwrap();

    for iface in ifaces.iter() {
        iface.poll();
    }
}

fn spawn_background_poll_thread(iface: Arc<Iface>) {
    let task_fn = move || {
        trace!("spawn background poll thread for {}", iface.name());

        let iface_ext = iface.ext();
        let wait_queue = iface_ext.polling_wait_queue();

        loop {
            let next_poll_at_ms = if let Some(next_poll_at_ms) = iface_ext.next_poll_at_ms() {
                next_poll_at_ms
            } else {
                wait_queue.wait_until(|| iface_ext.next_poll_at_ms())
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
                // If `iface_ext.next_poll_at_ms()` changes to an earlier time, we will end the
                // waiting.
                || (iface_ext.next_poll_at_ms()? < next_poll_at_ms).then_some(()),
                &duration,
            );
        }
    };

    // FIXME: remove the use of real-time priority.
    ThreadOptions::new(task_fn)
        .priority(Priority::default_real_time())
        .spawn();
}
