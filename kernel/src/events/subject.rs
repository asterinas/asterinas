// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use keyable_arc::KeyableWeak;
use ostd::sync::LocalIrqDisabled;

use super::{Events, EventsFilter, Observer};
use crate::prelude::*;

/// A subject that notifies interesting events to registered observers.
///
/// This type does not have any inner locks. Therefore, users need to maintain an outer lock to
/// obtain a mutable reference. Consequently, observers can break the atomic mode as long as the
/// outer lock also permits it.
pub struct Subject<E: Events>(BTreeSet<KeyableWeak<dyn Observer<E>>>);

impl<E: Events> Subject<E> {
    /// Creates an empty subject.
    pub const fn new() -> Self {
        Self(BTreeSet::new())
    }

    /// Registers an observer.
    ///
    /// A registered observer will get notified through its `on_events` method.
    pub fn register_observer(&mut self, observer: Weak<dyn Observer<E>>) {
        self.0.insert(KeyableWeak::from(observer));
    }

    /// Unregisters an observer.
    ///
    /// If such an observer is found, then the registered observer will be
    /// removed from the set and this method will return `true`. Otherwise,
    /// a `false` will be returned.
    pub fn unregister_observer(&mut self, observer: &Weak<dyn Observer<E>>) -> bool {
        self.0.remove(&KeyableWeak::from(observer.clone()))
    }

    /// Notifies events to all registered observers.
    ///
    /// It will remove the observers which have been freed.
    pub fn notify_observers(&mut self, events: &E) {
        self.0.retain(|observer| {
            if let Some(observer) = observer.upgrade() {
                observer.on_events(events);
                true
            } else {
                false
            }
        });
    }
}

impl<E: Events> Default for Subject<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// A synchronized subject that notifies interesting events to registered observers.
///
/// This type can be used via an immutable reference across threads. To enable this, the type
/// maintains registered observers in a spin lock. As a result, when called on events, all
/// registered observers should not break atomic mode. See also [`Subject`] if the condition may be
/// violated.
pub struct SyncSubject<E: Events, F: EventsFilter<E> = ()> {
    // A table that maintains all interesting observers.
    observers: SpinLock<BTreeMap<KeyableWeak<dyn Observer<E>>, F>, LocalIrqDisabled>,
    // To reduce lock contentions, we maintain a counter for the size of the table
    num_observers: AtomicUsize,
}

impl<E: Events, F: EventsFilter<E>> SyncSubject<E, F> {
    /// Creates an empty subject.
    pub const fn new() -> Self {
        Self {
            observers: SpinLock::new(BTreeMap::new()),
            num_observers: AtomicUsize::new(0),
        }
    }

    /// Registers an observer.
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
            // This `Acquire` pairs with the `Release` in `notify_observers`.
            self.num_observers.fetch_add(1, Ordering::Acquire);
        }
    }

    /// Unregisters an observer.
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

    /// Notifies events to all registered observers.
    ///
    /// It will remove the observers which have been freed.
    pub fn notify_observers(&self, events: &E) {
        // Fast path.
        //
        // Note: This must use `Release`, which pairs with `Acquire` in `register_observer`, to
        // ensure that even if this fast path is used, a concurrently registered observer will see
        // the event we want to notify.
        if self.num_observers.fetch_add(0, Ordering::Release) == 0 {
            return;
        }

        // Slow path: broadcast the new events to all observers.
        let mut num_freed = 0;
        let mut observers = self.observers.lock();
        observers.retain(|observer, filter| {
            if let Some(observer) = observer.upgrade() {
                if filter.filter(events) {
                    observer.on_events(events);
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
    }
}

impl<E: Events> Default for SyncSubject<E> {
    fn default() -> Self {
        Self::new()
    }
}
