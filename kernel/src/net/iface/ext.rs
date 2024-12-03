// SPDX-License-Identifier: MPL-2.0

use super::sched::PollScheduler;

pub struct BigtcpExt;

impl aster_bigtcp::ext::Ext for BigtcpExt {
    type ScheduleNextPoll = PollScheduler;
}
