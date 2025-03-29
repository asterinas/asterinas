// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::{Events, EventsFilter, Observer, Subject},
    prelude::*,
};

/// Represents possible actions for a UEvent
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Add,
    Remove,
    Offline,
    Online,
    Change,
    Move,
}

/// Represents a kernel event that can be exported to userspace.
/// FIXME: Currently, this is a stub implementation, because we don't support UEvent now.
/// It should be used with the trait `PseudoExt`.
#[derive(Debug, Clone, Copy)]
pub struct UEvent {
    action: Action,
}

impl UEvent {
    pub fn new(action: Action) -> Self {
        Self { action }
    }

    pub fn export(&self) -> String {
        match self.action {
            Action::Add => "ACTION=add\n",
            Action::Remove => "ACTION=remove\n",
            Action::Offline => "ACTION=offline\n",
            Action::Online => "ACTION=online\n",
            Action::Change => "ACTION=change\n",
            Action::Move => "ACTION=move\n",
        }
        .to_string()
    }
}

impl Events for UEvent {}

/// Filter for UEvents (currently filters out all events)
#[derive(Debug, Clone, Copy)]
pub struct UEventFilter;

impl EventsFilter<UEvent> for UEventFilter {
    fn filter(&self, event: &UEvent) -> bool {
        false
    }
}
