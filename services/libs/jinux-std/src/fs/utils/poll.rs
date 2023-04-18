use super::IoEvents;
use crate::events::Observer;
use crate::prelude::*;

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use jinux_frame::sync::WaitQueue;
use keyable_arc::KeyableArc;

/// A pollee maintains a set of active events, which can be polled with
/// pollers or be monitored with observers.
pub struct Pollee {
    inner: Arc<PolleeInner>,
}

struct PolleeInner {
    // A table that maintains all interesting pollers
    pollers: Mutex<BTreeMap<KeyableArc<dyn Observer<IoEvents>>, IoEvents>>,
    // For efficient manipulation, we use AtomicU32 instead of RwLock<IoEvents>
    events: AtomicU32,
    // To reduce lock contentions, we maintain a counter for the size of the table
    num_pollers: AtomicUsize,
}

impl Pollee {
    /// Creates a new instance of pollee.
    pub fn new(init_events: IoEvents) -> Self {
        let inner = PolleeInner {
            pollers: Mutex::new(BTreeMap::new()),
            events: AtomicU32::new(init_events.bits()),
            num_pollers: AtomicUsize::new(0),
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
    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let mask = mask | IoEvents::ALWAYS_POLL;

        // Fast path: return events immediately
        let revents = self.events() & mask;
        if !revents.is_empty() || poller.is_none() {
            return revents;
        }

        // Slow path: register the provided poller
        self.register_poller(poller.unwrap(), mask);

        // It is important to check events again to handle race conditions
        let revents = self.events() & mask;
        revents
    }

    fn register_poller(&self, poller: &Poller, mask: IoEvents) {
        let mut pollers = self.inner.pollers.lock();
        let is_new = {
            let observer = poller.observer();
            pollers.insert(observer, mask).is_none()
        };
        if is_new {
            let mut pollees = poller.inner.pollees.lock();
            pollees.push(Arc::downgrade(&self.inner));

            self.inner.num_pollers.fetch_add(1, Ordering::Release);
        }
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
    ///
    /// # Memory leakage
    ///
    /// Since an `Arc` for each observer is kept internally by a pollee,
    /// it is important for the user to call the `unregister_observer` method
    /// when the observer is no longer interested in the pollee. Otherwise,
    /// the observer will not be dropped.
    pub fn register_observer(&self, observer: Arc<dyn Observer<IoEvents>>, mask: IoEvents) {
        let mut pollers = self.inner.pollers.lock();
        let is_new = {
            let observer: KeyableArc<dyn Observer<IoEvents>> = observer.into();
            let mask = mask | IoEvents::ALWAYS_POLL;
            pollers.insert(observer, mask).is_none()
        };
        if is_new {
            self.inner.num_pollers.fetch_add(1, Ordering::Release);
        }
    }

    /// Unregister an IoEvents observer.
    ///
    /// If such an observer is found, then the registered observer will be
    /// removed from the pollee and returned as the return value. Otherwise,
    /// a `None` will be returned.
    pub fn unregister_observer(
        &self,
        observer: &Arc<dyn Observer<IoEvents>>,
    ) -> Option<Arc<dyn Observer<IoEvents>>> {
        let observer: KeyableArc<dyn Observer<IoEvents>> = observer.clone().into();
        let mut pollers = self.inner.pollers.lock();
        let observer = pollers
            .remove_entry(&observer)
            .map(|(observer, _)| observer.into());
        if observer.is_some() {
            self.inner.num_pollers.fetch_sub(1, Ordering::Relaxed);
        }
        observer
    }

    /// Add some events to the pollee's state.
    ///
    /// This method wakes up all registered pollers that are interested in
    /// the added events.
    pub fn add_events(&self, events: IoEvents) {
        self.inner.events.fetch_or(events.bits(), Ordering::Release);

        // Fast path
        if self.inner.num_pollers.load(Ordering::Relaxed) == 0 {
            return;
        }

        // Slow path: broadcast the new events to all pollers
        let pollers = self.inner.pollers.lock();
        pollers
            .iter()
            .filter(|(_, mask)| mask.intersects(events))
            .for_each(|(poller, mask)| poller.on_events(&(events & *mask)));
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
    inner: KeyableArc<PollerInner>,
}

struct PollerInner {
    // Use event counter to wait or wake up a poller
    event_counter: EventCounter,
    // All pollees that are interesting to this poller
    pollees: Mutex<Vec<Weak<PolleeInner>>>,
}

impl Poller {
    /// Constructs a new `Poller`.
    pub fn new() -> Self {
        let inner = PollerInner {
            event_counter: EventCounter::new(),
            pollees: Mutex::new(Vec::with_capacity(1)),
        };
        Self {
            inner: KeyableArc::new(inner),
        }
    }

    /// Wait until there are any interesting events happen since last `wait`.
    pub fn wait(&self) {
        self.inner.event_counter.read();
    }

    fn observer(&self) -> KeyableArc<dyn Observer<IoEvents>> {
        self.inner.clone() as KeyableArc<dyn Observer<IoEvents>>
    }
}

impl Observer<IoEvents> for PollerInner {
    fn on_events(&self, _events: &IoEvents) {
        self.event_counter.write();
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        let mut pollees = self.inner.pollees.lock();
        if pollees.len() == 0 {
            return;
        }

        let self_observer = self.observer();
        for weak_pollee in pollees.drain(..) {
            if let Some(pollee) = weak_pollee.upgrade() {
                let mut pollers = pollee.pollers.lock();
                let res = pollers.remove(&self_observer);
                assert!(res.is_some());
                drop(pollers);

                pollee.num_pollers.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

/// A counter for wait and wakeup.
struct EventCounter {
    counter: AtomicUsize,
    wait_queue: WaitQueue,
}

impl EventCounter {
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
            wait_queue: WaitQueue::new(),
        }
    }

    pub fn read(&self) -> usize {
        self.wait_queue.wait_until(|| {
            let val = self.counter.swap(0, Ordering::Relaxed);
            if val > 0 {
                Some(val)
            } else {
                None
            }
        })
    }

    pub fn write(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
        self.wait_queue.wake_one();
    }
}
