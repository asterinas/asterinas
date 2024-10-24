// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};

use ostd::sync::{Waiter, Waker};

use crate::{
    events::{IoEvents, Observer, Subject},
    prelude::*,
};

/// A pollee maintains a set of active events, which can be polled with
/// pollers or be monitored with observers.
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

    /// Returns the current events of the pollee filtered by the given event mask.
    ///
    /// If a poller is provided, the poller will start monitoring the pollee and receive event
    /// notification when the pollee receives interesting events.
    ///
    /// This operation is _atomic_ in the sense that if there are interesting events, either the
    /// events are returned or the poller is notified.
    pub fn poll(&self, mask: IoEvents, poller: Option<&mut AnyPoller>) -> IoEvents {
        let mask = mask | IoEvents::ALWAYS_POLL;

        // Register the provided poller.
        if let Some(poller) = poller {
            self.register_poller(poller, mask);
        }

        // Check events after the registration to prevent race conditions.
        self.events() & mask
    }

    fn register_poller(&self, poller: &mut AnyPoller, mask: IoEvents) {
        self.inner
            .subject
            .register_observer(poller.observer.clone(), mask);

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

/// A poller that is notified when its associated pollees have interesting events.
///
/// The observer type, `T`, should be a type that implements the [`Observer`] trait.
pub struct Poller<T> {
    // The event observer.
    observer: Arc<T>,
    // The inner with observer type erased.
    inner: AnyPoller,
}

impl<T: Observer<IoEvents> + 'static> Poller<T> {
    /// Constructs a new poller with the specified observer.
    pub fn with_observer(observer: T) -> Self {
        let observer = Arc::new(observer);
        let inner = AnyPoller::new(Arc::downgrade(&observer) as _);

        Self { observer, inner }
    }
}

impl<T> Poller<T> {
    /// Gets a reference to the observer.
    pub fn observer(&self) -> &Arc<T> {
        &self.observer
    }

    /// Gets a mutable reference to the type-erased poller.
    ///
    /// See [`AnyPoller`] for more details.
    pub fn as_dyn(&mut self) -> &mut AnyPoller {
        &mut self.inner
    }
}

/// A poller, but with the observer type erased.
///
/// This type is used to register the poller in [`Pollable::poll`]. The observer type is erased
/// because the [`Pollable`] trait must be kept object-safe.
pub struct AnyPoller {
    // The event observer.
    observer: Weak<dyn Observer<IoEvents>>,
    // The associated pollees.
    pollees: Vec<Weak<PolleeInner>>,
}

impl AnyPoller {
    /// Constructs a new poller with the observer.
    ///
    /// Note: It is a *logic error* to construct the multiple poller with the same observer (where
    /// "same" means [`Weak::ptr_eq`]). If possible, consider using [`Poller::with_observer`]
    /// instead.
    pub fn new(observer: Weak<dyn Observer<IoEvents>>) -> Self {
        Self {
            observer,
            pollees: Vec::new(),
        }
    }

    /// Resets the poller.
    ///
    /// The observer will be unregistered and will no longer receive events.
    pub fn reset(&mut self) {
        let observer = &self.observer;

        self.pollees
            .iter()
            .filter_map(Weak::upgrade)
            .for_each(|pollee| {
                pollee.subject.unregister_observer(observer);
            });
    }
}

impl Drop for AnyPoller {
    fn drop(&mut self) {
        self.reset();
    }
}

/// A poller that can be used to wait for some events.
pub struct WaitablePoller {
    poller: Poller<EventCounter>,
    waiter: Waiter,
}

impl WaitablePoller {
    /// Constructs a new poller to wait for interesting events.
    pub fn new() -> Self {
        let (waiter, event_counter) = EventCounter::new_pair();

        Self {
            poller: Poller::with_observer(event_counter),
            waiter,
        }
    }

    /// Gets a mutable reference to the type-erased poller.
    ///
    /// See [`AnyPoller`] for more details.
    pub fn as_dyn(&mut self) -> &mut AnyPoller {
        self.poller.as_dyn()
    }

    /// Waits until some interesting events happen since the last wait or until the timeout
    /// expires.
    ///
    /// The waiting process can be interrupted by a signal.
    pub fn wait(&self, timeout: Option<&Duration>) -> Result<()> {
        self.poller.observer().read(&self.waiter, timeout)?;
        Ok(())
    }
}

struct EventCounter {
    counter: AtomicUsize,
    waker: Arc<Waker>,
}

impl EventCounter {
    fn new_pair() -> (Waiter, Self) {
        let (waiter, waker) = Waiter::new_pair();

        (
            waiter,
            Self {
                counter: AtomicUsize::new(0),
                waker,
            },
        )
    }

    fn read(&self, waiter: &Waiter, timeout: Option<&Duration>) -> Result<usize> {
        let cond = || {
            let val = self.counter.swap(0, Ordering::Relaxed);
            if val > 0 {
                Some(val)
            } else {
                None
            }
        };

        if let Some(timeout) = timeout {
            waiter.pause_until_or_timeout(cond, timeout)
        } else {
            waiter.pause_until(cond)
        }
    }

    fn write(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
        self.waker.wake_up();
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
/// have access to the internal [`Pollee`], but there is a method that provides the same semantics
/// as [`Pollee::poll`] and we need to perform event-based operations using that method.
pub trait Pollable {
    /// Returns the interesting events now and monitors their occurrence in the future if the
    /// poller is provided.
    ///
    /// This method has the same semantics as [`Pollee::poll`].
    fn poll(&self, mask: IoEvents, poller: Option<&mut AnyPoller>) -> IoEvents;

    /// Waits for events and performs event-based operations.
    ///
    /// If a call to `cond()` succeeds or fails with an error code other than `EAGAIN`, the method
    /// will return whatever the call to `cond()` returns. Otherwise, the method will wait for some
    /// interesting events specified in `mask` to happen and try again.
    ///
    /// This method will fail with `ETIME` if the timeout is specified and the event does not occur
    /// before the timeout expires.
    ///
    /// The user must ensure that a call to `cond()` does not fail with `EAGAIN` when the
    /// interesting events occur. However, it is allowed to have spurious `EAGAIN` failures due to
    /// race conditions where the events are consumed by another thread.
    fn wait_events<F, R>(
        &self,
        mask: IoEvents,
        timeout: Option<&Duration>,
        mut cond: F,
    ) -> Result<R>
    where
        Self: Sized,
        F: FnMut() -> Result<R>,
    {
        // Fast path: Return immediately if the operation gives a result.
        match cond() {
            Err(err) if err.error() == Errno::EAGAIN => (),
            result => return result,
        }

        // Fast path: Return immediately if the timeout is zero.
        if timeout.is_some_and(|duration| duration.is_zero()) {
            return_errno_with_message!(Errno::ETIME, "the timeout expired");
        }

        // Wait until the event happens.
        let mut poller = WaitablePoller::new();
        if self.poll(mask, Some(poller.as_dyn())).is_empty() {
            poller.wait(timeout)?;
        }

        loop {
            // Try again after the event happens.
            match cond() {
                Err(err) if err.error() == Errno::EAGAIN => (),
                result => return result,
            };

            // Wait until the next event happens.
            //
            // FIXME: We need to update `timeout` since we have waited for some time.
            poller.wait(timeout)?;
        }
    }
}
