// SPDX-License-Identifier: MPL-2.0

/// A trait to provide the `schedule_next_poll` method for ifaces.
pub trait ScheduleNextPoll: Send + Sync {
    /// Schedules the next poll at the specific time.
    ///
    /// This is invoked with the time at which the next poll should be performed, or `None` if no
    /// next poll is required. It's up to the caller to determine the mechanism to ensure that the
    /// next poll happens at the right time (e.g. by setting a timer).
    fn schedule_next_poll(&self, ms: Option<u64>);
}
