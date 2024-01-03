// SPDX-License-Identifier: MPL-2.0

use super::{Events, EventsFilter};

crate::bitflags! {
    pub struct IoEvents: u32 {
        const IN    = 0x0001;
        const PRI   = 0x0002;
        const OUT   = 0x0004;
        const ERR   = 0x0008;
        const HUP   = 0x0010;
        const NVAL  = 0x0020;
        const RDHUP = 0x2000;
        /// Events that are always polled even without specifying them.
        const ALWAYS_POLL = Self::ERR.bits | Self::HUP.bits;
    }
}

impl Events for IoEvents {}

impl EventsFilter<IoEvents> for IoEvents {
    fn filter(&self, events: &IoEvents) -> bool {
        self.intersects(*events)
    }
}
