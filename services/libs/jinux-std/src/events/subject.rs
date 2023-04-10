use crate::prelude::*;

use super::{Events, Observer};

/// A Subject notify interesting events to registered observers.
pub struct Subject<E: Events> {
    observers: Mutex<Vec<Weak<dyn Observer<E>>>>,
}

impl<E: Events> Subject<E> {
    pub fn new() -> Self {
        Self {
            observers: Mutex::new(Vec::new()),
        }
    }

    /// Register an observer.
    pub fn register_observer(&self, observer: Weak<dyn Observer<E>>) {
        let mut observers = self.observers.lock();
        observers.push(observer);
    }

    /// Unregister an observer.
    pub fn unregister_observer(&self, observer: Weak<dyn Observer<E>>) {
        let mut observers = self.observers.lock();
        observers.retain(|e| !Weak::ptr_eq(&e, &observer));
    }

    /// Notify events to all registered observers.
    /// It will remove the observers which have been freed.
    pub fn notify_observers(&self, events: &E) {
        let mut observers = self.observers.lock();
        let mut idx = 0;
        while idx < observers.len() {
            if let Some(observer) = observers[idx].upgrade() {
                observer.on_events(events);
                idx += 1;
            } else {
                observers.remove(idx);
            }
        }
    }
}
