// SPDX-License-Identifier: MPL-2.0

use super::sched::PollScheduler;

pub struct Ext;

impl aster_bigtcp::ext::Ext for Ext {
    type ScheduleNextPoll = PollScheduler;
}
