// SPDX-License-Identifier: MPL-2.0

use super::{sig_mask::SigSet, sig_num::SigNum};
use crate::{
    events::{Events, EventsFilter},
    prelude::*,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigEvents(SigNum);

impl SigEvents {
    pub fn new(sig_num: SigNum) -> Self {
        Self(sig_num)
    }
}

impl Events for SigEvents {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigEventsFilter(SigSet);

impl SigEventsFilter {
    pub fn new(mask: SigSet) -> Self {
        Self(mask)
    }
}

impl EventsFilter<SigEvents> for SigEventsFilter {
    fn filter(&self, event: &SigEvents) -> bool {
        !self.0.contains(event.0)
    }
}
