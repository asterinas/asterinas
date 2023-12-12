use crate::events::{Events, EventsSelector};
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
pub struct SigEventsSelector(SigMask);

impl SigEventsSelector {
    pub fn new(mask: SigMask) -> Self {
        Self(mask)
    }
}

impl EventsSelector<SigEvents> for SigEventsSelector {
    fn select(&self, event: &SigEvents) -> bool {
        !self.0.contains(event.0)
    }
}
