use super::Events;

/// An observer for events.
///
/// In a sense, event observers are just a fancy form of callback functions.
/// An observer's `on_events` methods are supposed to be called when
/// some events that are interesting to the observer happen.
pub trait Observer<E: Events>: Send + Sync {
    /// Notify the observer that some interesting events happen.
    fn on_events(&self, events: &E);
}
