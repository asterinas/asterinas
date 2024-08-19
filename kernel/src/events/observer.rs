// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use super::Events;

/// An observer for events.
///
/// In a sense, event observers are just a fancy form of callback functions.
/// An observer's `on_events` methods are supposed to be called when
/// some events that are interesting to the observer happen.
///
/// # The no-op observer
///
/// The unit type `()` can serve as a no-op observer.
/// It implements `Observer<E>` for any events type `E`,
/// with an `on_events` method that simply does nothing.
///
/// It can be used to create an empty `Weak`, as shown in the example below.
/// Using the unit type is necessary, as creating an empty `Weak` needs to
/// have a sized type (e.g. the unit type).
///
/// # Examples
///
/// ```
/// use alloc::sync::Weak;
/// use crate::events::Observer;
///
/// let empty: Weak<dyn Observer<()>> = Weak::<()>::new();
/// assert!(empty.upgrade().is_empty());
/// ```
pub trait Observer<E: Events>: Send + Sync {
    /// Notify the observer that some interesting events happen.
    fn on_events(&self, events: &E);
}

impl<E: Events> Observer<E> for () {
    fn on_events(&self, events: &E) {}
}
