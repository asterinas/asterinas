// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use keyable_arc::KeyableWeak;
use ostd::sync::LocalIrqDisabled;

use super::{Events, EventsFilter, Observer};
use crate::prelude::*;

/// A Subject notifies interesting events to registered observers.
pub struct Subject<E: Events, F: EventsFilter<E> = ()> {
    // A table that maintains all interesting observers.
    observers: SpinLock<BTreeMap<KeyableWeak<dyn Observer<E>>, F>, LocalIrqDisabled>,
    // To reduce lock contentions, we maintain a counter for the size of the table
    num_observers: AtomicUsize,
}

impl<E: Events, F: EventsFilter<E>> Subject<E, F> {
    pub const fn new() -> Self {
        Self {
            observers: SpinLock::new(BTreeMap::new()),
            num_observers: AtomicUsize::new(0),
        }
    }
    /// Register an observer.
    ///
    /// A registered observer will get notified through its `on_events` method.
    /// If events `filter` is provided, only filtered events will notify the observer.
    ///
    /// If the given observer has already been registered, then its registered events
    /// filter will be updated.
    pub fn register_observer(&self, observer: Weak<dyn Observer<E>>, filter: F) {
        let mut observers = self.observers.lock();
        let is_new = {
            let observer: KeyableWeak<dyn Observer<E>> = observer.into();
            observers.insert(observer, filter).is_none()
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
        let mut active_observers = Vec::new();
        let mut num_freed = 0;
        let mut observers = self.observers.lock();
        observers.retain(|observer, filter| {
            if let Some(observer) = observer.upgrade() {
                if filter.filter(events) {
                    // XXX: Mind the performance impact when there comes many active observers
                    active_observers.push(observer.clone());
                }
                true
            } else {
                num_freed += 1;
                false
            }
        });
        if num_freed > 0 {
            self.num_observers.fetch_sub(num_freed, Ordering::Relaxed);
        }
        drop(observers);

        for observer in active_observers {
            observer.on_events(events);
        }
    }
}

impl<E: Events> Default for Subject<E> {
    fn default() -> Self {
        Self::new()
    }
}
