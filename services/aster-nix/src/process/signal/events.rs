// SPDX-License-Identifier: MPL-2.0

use crate::events::{Events, EventsFilter};
use crate::prelude::*;

use super::sig_mask::SigMask;
use super::sig_num::SigNum;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigEvents(SigNum);

impl SigEvents {
    pub fn new(sig_num: SigNum) -> Self {
        Self(sig_num)
    }
}

impl Events for SigEvents {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigEventsFilter(SigMask);

impl SigEventsFilter {
    pub fn new(mask: SigMask) -> Self {
        Self(mask)
    }
}

impl EventsFilter<SigEvents> for SigEventsFilter {
    fn filter(&self, event: &SigEvents) -> bool {
        !self.0.contains(event.0)
    }
}
