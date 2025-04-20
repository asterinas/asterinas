// SPDX-License-Identifier: MPL-2.0

// Imports needed for the remaining structs/enums
// use super::node::SysObj; // SysObj needed if publish_event is uncommented
use alloc::{string::String, vec::Vec}; // Import standard types
use core::fmt::Debug;

use super::SysStr; // Import from parent (lib.rs) // For derive(Debug)

// --- Event Hub ---
/*
/// An event hub is where one can publish and subscribe events in a `SysTree`.
///
/// Requires implementations for Subject, Observer, EventsFilter traits.
#[derive(Debug)] // Added Debug derive
pub struct SysEventHub {
    subject: Subject<SysEvent, SysEventSelector>,
}

impl SysEventHub {
    pub const fn new() -> Self {
        Self {
            subject: Subject::new(),
        }
    }

    pub fn publish_event(&self,
        obj: &dyn SysObj, // Requires SysObj trait
        action: SysEventAction,
        details: Vec<SysEventKv> // Requires Vec
    ) {
        // Requires obj.path() -> Option<String>
        let Some(path) = obj.path() else {
            // The object is not attached to the systree, yet.
            // We do not allow unattached object to publish events.
            return;
        };

        let event = SysEvent::new(action, path, details);
        self.subject.notify_observers(&event); // Requires Subject::notify_observers
    }

    pub fn register_observer(&self,
        observer: Weak<dyn Observer<SysEvent>>, // Requires Weak, Observer
        filter: SysEventSelector
    ) /* -> Option<()> */ { // Original had Option<> which is invalid syntax
        // self.subject.register_observer(observer, filter).unwrap() // Requires Subject::register_observer
        todo!()
    }

    pub fn unregister_observer(&self, observer: Weak<dyn Observer<SysEvent>>) // Requires Weak, Observer
        -> Option<Weak<dyn Observer<SysEvent>>> // Requires Weak, Observer
    {
        self.subject.unregister_observer(observer) // Requires Subject::unregister_observer
    }
}
*/

// --- Event Selector ---
/*
/// A selector (i.e., a filter) for events that occur in the `SysTree`.
#[derive(Debug, Clone, Copy)] // Added derives
pub enum SysEventSelector {
    // Select all events.
    All,
    // Select only events of a specific action.
    Action(SysEventAction),
}

// Requires EventsFilter trait definition
impl EventsFilter<SysEvent> for SysEventSelector {
    fn filter(&self, event: &SysEvent) -> bool {
        match self {
            Self::All => true,
            Self::Action(action) => *action == event.action(), // Deref action
        }
    }
}
*/

// --- Event Definitions ---

/// An event happens in the `SysTree`.
///
/// An event consists of three components:
/// * Which _action_ triggers the event (`self.action()`);
/// * On which _path_ the event occurs (`self.path()`);
/// * More _details_ about the event, encoded as key-value pairs (`self.details`).
#[derive(Clone, Debug)]
pub struct SysEvent {
    // Mandatory info
    //
    // Which action happens
    action: SysEventAction,
    // Where the event originates from
    path: String, // Requires alloc::string::String
    // Optional details
    details: Vec<SysEventKv>, // Requires alloc::vec::Vec
}

impl SysEvent {
    pub fn new(action: SysEventAction, path: String, details: Vec<SysEventKv>) -> Self {
        // Requires String, Vec
        Self {
            action,
            path, // Requires String
            details,
        }
    }

    pub fn action(&self) -> SysEventAction {
        self.action
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn details(&self) -> &[SysEventKv] {
        // Requires SysEventKv
        &self.details
    }
}

/// A key-value pair of strings, which encodes information about an `SysEvent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysEventKv {
    pub key: SysStr,   // Requires SysStr
    pub value: SysStr, // Requires SysStr
}

/// The action of an `SysEvent`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SysEventAction {
    /// Add a new node in the `SysTree`.
    Add,
    /// Remove an existing node from the `SysTree`.
    Remove,
    /// Change a node in the `SysTree`.
    Change,
}
