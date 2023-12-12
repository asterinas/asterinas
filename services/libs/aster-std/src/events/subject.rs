use crate::prelude::*;

use core::sync::atomic::{AtomicUsize, Ordering};
use keyable_arc::KeyableWeak;

use super::{Events, EventsSelector, Observer};

/// A Subject notifies interesting events to registered observers.
pub struct Subject<E: Events, S: EventsSelector<E> = ()> {
    // A table that maintains all interesting observers.
    observers: Mutex<BTreeMap<KeyableWeak<dyn Observer<E>>, S>>,
    // To reduce lock contentions, we maintain a counter for the size of the table
    num_observers: AtomicUsize,
}

impl<E: Events, S: EventsSelector<E>> Subject<E, S> {
    pub const fn new() -> Self {
        Self {
            observers: Mutex::new(BTreeMap::new()),
            num_observers: AtomicUsize::new(0),
        }
    }
    /// Register an observer.
    ///
    /// A registered observer will get notified through its `on_events` method.
    /// If events `selector` is provided, only selected events will notify the observer.
    ///
    /// If the given observer has already been registered, then its registered events
    /// selector will be updated.
    pub fn register_observer(&self, observer: Weak<dyn Observer<E>>, selector: S) {
        let mut observers = self.observers.lock();
        let is_new = {
            let observer: KeyableWeak<dyn Observer<E>> = observer.into();
            observers.insert(observer, selector).is_none()
        };
        if is_new {
            self.num_observers.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Unregister an observer.
    ///
    /// If such an observer is found, then the registered observer will be
    /// removed from the subject and returned as the return value. Otherwise,
    /// a `None` will be returned.
    pub fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<E>>,
    ) -> Option<Weak<dyn Observer<E>>> {
        let observer: KeyableWeak<dyn Observer<E>> = observer.clone().into();
        let mut observers = self.observers.lock();
        let observer = observers
            .remove_entry(&observer)
            .map(|(observer, _)| observer.into());
        if observer.is_some() {
            self.num_observers.fetch_sub(1, Ordering::Relaxed);
        }
        observer
    }

    /// Notify events to all registered observers.
    ///
    /// It will remove the observers which have been freed.
    pub fn notify_observers(&self, events: &E) {
        // Fast path.
        if self.num_observers.load(Ordering::Relaxed) == 0 {
            return;
        }

        // Slow path: broadcast the new events to all observers.
        let mut observers = self.observers.lock();
        observers.retain(|observer, selector| {
            if let Some(observer) = observer.upgrade() {
                if selector.select(events) {
                    observer.on_events(events);
                }
                true
            } else {
                self.num_observers.fetch_sub(1, Ordering::Relaxed);
                false
            }
        });
    }
}
