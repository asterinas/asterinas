// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};

use crate::{
    events::{IoEvents, Observer, Subject},
    prelude::*,
    process::signal::Pauser,
};

/// A pollee maintains a set of active events, which can be polled with
/// pollers or be monitored with observers.
#[derive(Clone)]
pub struct Pollee {
    inner: Arc<PolleeInner>,
}

struct PolleeInner {
    // A subject which is monitored with pollers.
    subject: Subject<IoEvents, IoEvents>,
    // For efficient manipulation, we use AtomicU32 instead of RwLock<IoEvents>.
    events: AtomicU32,
}

impl Pollee {
    /// Creates a new instance of pollee.
    pub fn new(init_events: IoEvents) -> Self {
        let inner = PolleeInner {
            subject: Subject::new(),
            events: AtomicU32::new(init_events.bits()),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Returns the current events of the pollee given an event mask.
    ///
    /// If no interesting events are polled and a poller is provided, then
    /// the poller will start monitoring the pollee and receive event
    /// notification once the pollee gets any interesting events.
    ///
    /// This operation is _atomic_ in the sense that either some interesting
    /// events are returned or the poller is registered (if a poller is provided).
    pub fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        let mask = mask | IoEvents::ALWAYS_POLL;

        // Fast path: return events immediately
        let revents = self.events() & mask;
        if !revents.is_empty() || poller.is_none() {
            return revents;
        }

        // Register the provided poller.
        self.register_poller(poller.unwrap(), mask);

        // It is important to check events again to handle race conditions
        self.events() & mask
    }

    fn register_poller(&self, poller: &mut Poller, mask: IoEvents) {
        self.inner
            .subject
            .register_observer(poller.observer(), mask);

        poller.pollees.push(Arc::downgrade(&self.inner));
    }

    /// Register an IoEvents observer.
    ///
    /// A registered observer will get notified (through its `on_events` method)
    /// every time new events specified by the `mask` argument happen on the
    /// pollee (through the `add_events` method).
    ///
    /// If the given observer has already been registered, then its registered
    /// event mask will be updated.
    ///
    /// Note that the observer will always get notified of the events in
    /// `IoEvents::ALWAYS_POLL` regardless of the value of `mask`.
    pub fn register_observer(&self, observer: Weak<dyn Observer<IoEvents>>, mask: IoEvents) {
        let mask = mask | IoEvents::ALWAYS_POLL;
        self.inner.subject.register_observer(observer, mask);
    }

    /// Unregister an IoEvents observer.
    ///
    /// If such an observer is found, then the registered observer will be
    /// removed from the pollee and returned as the return value. Otherwise,
    /// a `None` will be returned.
    pub fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.inner.subject.unregister_observer(observer)
    }

    /// Add some events to the pollee's state.
    ///
    /// This method wakes up all registered pollers that are interested in
    /// the added events.
    pub fn add_events(&self, events: IoEvents) {
        self.inner.events.fetch_or(events.bits(), Ordering::Release);
        self.inner.subject.notify_observers(&events);
    }

    /// Remove some events from the pollee's state.
    ///
    /// This method will not wake up registered pollers even when
    /// the pollee still has some interesting events to the pollers.
    pub fn del_events(&self, events: IoEvents) {
        self.inner
            .events
            .fetch_and(!events.bits(), Ordering::Release);
    }

    /// Reset the pollee's state.
    ///
    /// Reset means removing all events on the pollee.
    pub fn reset_events(&self) {
        self.inner
            .events
            .fetch_and(!IoEvents::all().bits(), Ordering::Release);
    }

    fn events(&self) -> IoEvents {
        let event_bits = self.inner.events.load(Ordering::Acquire);
        IoEvents::from_bits(event_bits).unwrap()
    }
}

/// A poller gets notified when its associated pollees have interesting events.
pub struct Poller {
    // Use event counter to wait or wake up a poller
    event_counter: Arc<EventCounter>,
    // All pollees that are interesting to this poller
    pollees: Vec<Weak<PolleeInner>>,
}

impl Default for Poller {
    fn default() -> Self {
        Self::new()
    }
}

impl Poller {
    /// Constructs a new `Poller`.
    pub fn new() -> Self {
        Self {
            event_counter: Arc::new(EventCounter::new()),
            pollees: Vec::new(),
        }
    }

    /// Wait until there are any interesting events happen since last `wait`. The `wait`
    /// can be interrupted by signal.
    pub fn wait(&self) -> Result<()> {
        self.event_counter.read(None)?;
        Ok(())
    }

    /// Wait until there are any interesting events happen since last `wait` or a given timeout
    /// is expired. This method can be interrupted by signal.
    pub fn wait_timeout(&self, timeout: &Duration) -> Result<()> {
        self.event_counter.read(Some(timeout))?;
        Ok(())
    }

    fn observer(&self) -> Weak<dyn Observer<IoEvents>> {
        Arc::downgrade(&self.event_counter) as _
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        let observer = self.observer();

        self.pollees
            .iter()
            .filter_map(Weak::upgrade)
            .for_each(|pollee| {
                pollee.subject.unregister_observer(&observer);
            });
    }
}

/// A counter for wait and wakeup.
struct EventCounter {
    counter: AtomicUsize,
    pauser: Arc<Pauser>,
}

impl EventCounter {
    pub fn new() -> Self {
        let pauser = Pauser::new();

        Self {
            counter: AtomicUsize::new(0),
            pauser,
        }
    }

    pub fn read(&self, timeout: Option<&Duration>) -> Result<usize> {
        let cond = || {
            let val = self.counter.swap(0, Ordering::Relaxed);
            if val > 0 {
                Some(val)
            } else {
                None
            }
        };

        if let Some(timeout) = timeout {
            self.pauser.pause_until_or_timeout(cond, timeout)
        } else {
            self.pauser.pause_until(cond)
        }
    }

    pub fn write(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
        self.pauser.resume_one();
    }
}

impl Observer<IoEvents> for EventCounter {
    fn on_events(&self, _events: &IoEvents) {
        self.write();
    }
}

/// The `Pollable` trait allows for waiting for events and performing event-based operations.
///
/// Implementors are required to provide a method, [`Pollable::poll`], which is usually implemented
/// by simply calling [`Pollee::poll`] on the internal [`Pollee`]. This trait provides another
/// method, [`Pollable::wait_events`], to allow waiting for events and performing operations
/// according to the events.
///
/// This trait is added instead of creating a new method in [`Pollee`] because sometimes we do not
/// have access to the internal [`Pollee`], but there is a method that provides the same sematics
/// as [`Pollee::poll`] and we need to perform event-based operations using that method.
pub trait Pollable {
    /// Returns the interesting events if there are any, or waits for them to happen if there are
    /// none.
    ///
    /// This method has the same semantics as [`Pollee::poll`].
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents;

    /// Waits for events and performs event-based operations.
    ///
    /// If a call to `cond()` succeeds or fails with an error code other than `EAGAIN`, the method
    /// will return whatever the call to `cond()` returns. Otherwise, the method will wait for some
    /// interesting events specified in `mask` to happen and try again.
    ///
    /// The user must ensure that a call to `cond()` does not fail with `EAGAIN` when the
    /// interesting events occur. However, it is allowed to have spurious `EAGAIN` failures due to
    /// race conditions where the events are consumed by another thread.
    fn wait_events<F, R>(&self, mask: IoEvents, mut cond: F) -> Result<R>
    where
        Self: Sized,
        F: FnMut() -> Result<R>,
    {
        let mut poller = Poller::new();

        loop {
            match cond() {
                Err(err) if err.error() == Errno::EAGAIN => (),
                result => return result,
            };

            let events = self.poll(mask, Some(&mut poller));
            if !events.is_empty() {
                continue;
            }

            // TODO: Support timeout
            poller.wait()?;
        }
    }
}
