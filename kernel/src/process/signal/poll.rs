// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
    time::Duration,
};

use ostd::{
    sync::{Waiter, Waker},
    task::Task,
};

use crate::{
    events::{IoEvents, Observer, Subject},
    prelude::*,
};

/// A pollee represents any I/O object (e.g., a file or socket) that can be polled.
///
/// `Pollee` provides a standard mechanism to allow
/// 1. An I/O object to maintain its I/O readiness; and
/// 2. An interested part to poll the object's I/O readiness.
///
/// To use the pollee correctly, you must follow the rules below carefully:
///  * [`Pollee::notify`] needs to be called whenever a new event arrives.
///  * [`Pollee::invalidate`] needs to be called whenever an old event disappears and no new event
///    arrives.
///
/// Then, [`Pollee::poll_with`] can allow you to register a [`Poller`] to wait for certain events,
/// or register a [`PollAdaptor`] to be notified when certain events occur.
pub struct Pollee {
    inner: Arc<PolleeInner>,
}

const INV_STATE: isize = -1;

struct PolleeInner {
    /// A subject which is monitored with pollers.
    subject: Subject<IoEvents, IoEvents>,
    /// A state that describes how events are cached in the pollee.
    ///
    /// The meaning of this field depends on its value:
    ///
    /// * A non-negative value represents cached events. The events are guaranteed to be
    ///   up-to-date, i.e., no one has called [`Pollee::notify`] or [`Pollee::invalidate`] since we
    ///   started checking the events.
    ///
    /// * A value of [`INV_STATE`] means no cached events. We may have previously cached some
    ///   events, but they are no longer valid due to calls of [`Pollee::notify`] or
    ///   [`Pollee::invalidate`].
    ///
    /// * A negative value other than [`INV_STATE`] represents a [`Task`] that is currently
    ///   checking events. When the task has finished checking and the state is neither invalidated
    ///   nor overwritten by another task checking events, the state can be used to cache the
    ///   checked events.
    state: AtomicIsize,
}

impl Default for Pollee {
    fn default() -> Self {
        Self::new()
    }
}

impl Pollee {
    /// Creates a new pollee.
    pub fn new() -> Self {
        let inner = PolleeInner {
            subject: Subject::new(),
            state: AtomicIsize::new(INV_STATE),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Returns the current events filtered by the given event mask.
    ///
    /// If a poller is provided, the poller will start monitoring the pollee and receive event
    /// notification when the pollee receives interesting events.
    ///
    /// This operation is _atomic_ in the sense that if there are interesting events, either the
    /// events are returned or the poller is notified.
    ///
    /// The above statement about atomicity is true even if `check` contains race conditions (and
    /// in fact it always will, because even if it holds a lock, the lock will be released when
    /// `check` returns).
    pub fn poll_with<F>(
        &self,
        mask: IoEvents,
        poller: Option<&mut PollHandle>,
        check: F,
    ) -> IoEvents
    where
        F: FnOnce() -> IoEvents,
    {
        let mask = mask | IoEvents::ALWAYS_POLL;

        // Register the provided poller.
        if let Some(poller) = poller {
            self.register_poller(poller, mask);
        }

        // Return the cached events, if any.
        let events = self.inner.state.load(Ordering::Acquire);
        if events >= 0 {
            return IoEvents::from_bits_truncate(events as _) & mask;
        }

        // If we know some task is checking the events, let it finish.
        if events != INV_STATE {
            return check() & mask;
        }

        // We will store `task_ptr` in `state` to indicate that we're checking the events. But we
        // need to make sure it's a negative value.
        const {
            use ostd::mm::KERNEL_VADDR_RANGE;
            assert!((KERNEL_VADDR_RANGE.start as isize) < 0);
        }
        let task_ptr = Task::current().unwrap().as_ref() as *const _ as isize;

        // Store `task_ptr` in `state` to indicate we're checking the events.
        //
        // Note that:
        // * If there are race conditions, `state` may contain something other than `INV_STATE` (as
        //   checked above), but that's okay.
        // * Given the first point, we only need to do a store here. However, we need the `Acquire`
        //   order, which forces us to do a `swap` operation. We ignore the returned value to allow
        //   the compiler to produce better assembly code.
        let _ = self.inner.state.swap(task_ptr, Ordering::Acquire);

        // Check events after the registration to prevent race conditions.
        let new_events = check();

        // If this `compare_exchange_weak` succeeds, we can guarantee that we are the only task
        // trying to cache the checked events, and that the events are not invalidated in the
        // middle, so we can cache them with confidence.
        //
        // Otherwise, we cache nothing, but returning the obsolete events is still okay.
        let _ = self.inner.state.compare_exchange_weak(
            task_ptr,
            new_events.bits() as _,
            Ordering::Release,
            Ordering::Relaxed,
        );

        // Return the events filtered by the mask.
        new_events & mask
    }

    fn register_poller(&self, poller: &mut PollHandle, mask: IoEvents) {
        self.inner
            .subject
            .register_observer(poller.observer.clone(), mask);

        poller.pollees.push(Arc::downgrade(&self.inner));
    }

    /// Notifies pollers of some events.
    ///
    /// This method invalidates the (internal) cached events and wakes up all registered pollers
    /// that are interested in the events.
    ///
    /// This method should be called whenever new events arrive. The events can be spurious. This
    /// way, the caller can avoid expensive calculations and simply add all possible ones.
    pub fn notify(&self, events: IoEvents) {
        self.invalidate();

        self.inner.subject.notify_observers(&events);
    }

    /// Invalidates the (internal) cached events.
    ///
    /// This method should be called whenever old events disappear but no new events arrive. The
    /// invalidation can be spurious, so the caller can avoid complex calculations and simply
    /// invalidate even if no events disappear.
    pub fn invalidate(&self) {
        // The memory order must be `Release`, so that the reader is guaranteed to see the changes
        // that trigger the invalidation.
        self.inner.state.store(INV_STATE, Ordering::Release);
    }
}

/// An opaque handle that can be used as an argument of the [`Pollable::poll`] method.
///
/// This type can represent an entity of [`PollAdaptor`] or [`Poller`], which is done via the
/// [`PollAdaptor::as_handle_mut`] and [`Poller::as_handle_mut`] methods.
///
/// When this handle is dropped or reset (via [`PollHandle::reset`]), the entity will no longer be
/// notified of the events from the pollee.
pub struct PollHandle {
    // The event observer.
    observer: Weak<dyn Observer<IoEvents>>,
    // The associated pollees.
    pollees: Vec<Weak<PolleeInner>>,
}

impl PollHandle {
    /// Constructs a new handle with the observer.
    ///
    /// Note: It is a *logic error* to construct the multiple handles with the same observer (where
    /// "same" means [`Weak::ptr_eq`]). If possible, consider using [`PollAdaptor::with_observer`]
    /// instead.
    pub fn new(observer: Weak<dyn Observer<IoEvents>>) -> Self {
        Self {
            observer,
            pollees: Vec::new(),
        }
    }

    /// Resets the handle.
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

impl Drop for PollHandle {
    fn drop(&mut self) {
        self.reset();
    }
}

/// An adaptor to make an [`Observer`] usable for [`Pollable::poll`].
///
/// Normally, [`Pollable::poll`] accepts a [`Poller`] which is used to wait for events. By using
/// this adaptor, it is possible to use any [`Observer`] with [`Pollable::poll`]. The observer will
/// be notified whenever there are new events.
pub struct PollAdaptor<O> {
    // The event observer.
    observer: Arc<O>,
    // The inner with observer type erased.
    inner: PollHandle,
}

impl<O: Observer<IoEvents> + 'static> PollAdaptor<O> {
    /// Constructs a new adaptor with the specified observer.
    pub fn with_observer(observer: O) -> Self {
        let observer = Arc::new(observer);
        let inner = PollHandle::new(Arc::downgrade(&observer) as _);

        Self { observer, inner }
    }
}

impl<O> PollAdaptor<O> {
    /// Gets a reference to the observer.
    pub fn observer(&self) -> &Arc<O> {
        &self.observer
    }

    /// Returns a mutable reference of [`PollHandle`].
    pub fn as_handle_mut(&mut self) -> &mut PollHandle {
        &mut self.inner
    }
}

/// A poller that can be used to wait for some events.
pub struct Poller {
    poller: PollAdaptor<EventCounter>,
    waiter: Waiter,
}

impl Poller {
    /// Constructs a new poller to wait for interesting events.
    pub fn new() -> Self {
        let (waiter, event_counter) = EventCounter::new_pair();

        Self {
            poller: PollAdaptor::with_observer(event_counter),
            waiter,
        }
    }

    /// Returns a mutable reference of [`PollHandle`].
    pub fn as_handle_mut(&mut self) -> &mut PollHandle {
        self.poller.as_handle_mut()
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

        waiter.pause_until_or_timeout(cond, timeout)
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
/// by simply calling [`Pollable::poll`] on the internal [`Pollee`]. This trait provides another
/// method, [`Pollable::wait_events`], to allow waiting for events and performing operations
/// according to the events.
///
/// This trait is added instead of creating a new method in [`Pollee`] because sometimes we do not
/// have access to the internal [`Pollee`], but there is a method that provides the same semantics
/// as [`Pollable::poll`] and we need to perform event-based operations using that method.
pub trait Pollable {
    /// Returns the interesting events now and monitors their occurrence in the future if the
    /// poller is provided.
    ///
    /// This method has the same semantics as [`Pollee::poll_with`].
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;

    /// Waits for events and performs event-based operations.
    ///
    /// If a call to `try_op()` succeeds or fails with an error code other than `EAGAIN`, the
    /// method will return whatever the call to `try_op()` returns. Otherwise, the method will wait
    /// for some interesting events specified in `mask` to happen and try again.
    ///
    /// This method will fail with `ETIME` if the timeout is specified and the event does not occur
    /// before the timeout expires.
    ///
    /// The user must ensure that a call to `try_op()` does not fail with `EAGAIN` when the
    /// interesting events occur. However, it is allowed to have spurious `EAGAIN` failures due to
    /// race opitions where the events are consumed by another thread.
    fn wait_events<F, R>(
        &self,
        mask: IoEvents,
        timeout: Option<&Duration>,
        mut try_op: F,
    ) -> Result<R>
    where
        Self: Sized,
        F: FnMut() -> Result<R>,
    {
        // Fast path: Return immediately if the operation gives a result.
        match try_op() {
            Err(err) if err.error() == Errno::EAGAIN => (),
            result => return result,
        }

        // Fast path: Return immediately if the timeout is zero.
        if timeout.is_some_and(|duration| duration.is_zero()) {
            return_errno_with_message!(Errno::ETIME, "the timeout expired");
        }

        // Wait until the event happens.
        let mut poller = Poller::new();
        if self.poll(mask, Some(poller.as_handle_mut())).is_empty() {
            poller.wait(timeout)?;
        }

        loop {
            // Try again after the event happens.
            match try_op() {
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

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn test_notify_before() {
        let pollee = Pollee::new();

        pollee.notify(IoEvents::OUT);

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::IN),
            // This is allowed, as we invoke the checking closure.
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            // This is allowed, as the cached state is still valid.
            IoEvents::IN
        );
    }

    #[ktest]
    fn test_notify_middle() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                pollee.notify(IoEvents::OUT);
                IoEvents::IN
            }),
            // This is allowed, as we invoke the checking closure.
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            // This is allowed, as we invoke the checking closure.
            //
            // Reusing the cached state is NOT allowed as we've been notified above.
            IoEvents::OUT
        );
    }

    #[ktest]
    fn test_notify_after() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::IN),
            // This is allowed, as we invoke the checking closure.
            IoEvents::IN
        );

        pollee.notify(IoEvents::OUT);

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            // This is allowed, as we invoke the checking closure.
            //
            // Reusing the cached state is NOT allowed as we've been notified above.
            IoEvents::OUT
        );
    }

    #[ktest]
    fn test_nested_notify_before() {
        let pollee = Pollee::new();

        pollee.notify(IoEvents::OUT);

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                assert_eq!(
                    pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
                    // This is allowed, as we invoke the checking closure.
                    IoEvents::OUT
                );
                IoEvents::IN
            }),
            // This is allowed, as we invoke the checking closure.
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            // This is allowed, as the cached state is still valid.
            IoEvents::IN
        );
    }

    #[ktest]
    fn test_nested_notify_between() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                pollee.notify(IoEvents::OUT);
                assert_eq!(
                    pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
                    // This is allowed, as we invoke the checking closure.
                    IoEvents::OUT
                );
                IoEvents::IN
            }),
            // This is allowed, as we invoke the checking closure.
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            // This is allowed, as we invoke the checking closure.
            //
            // Reusing the cached state is NOT allowed as we've been notified above.
            IoEvents::OUT
        );
    }

    #[ktest]
    fn test_nested_notify_inside() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                assert_eq!(
                    pollee.poll_with(IoEvents::all(), None, || {
                        pollee.notify(IoEvents::OUT);
                        IoEvents::OUT
                    }),
                    // This is allowed, as we invoke the checking closure.
                    IoEvents::OUT
                );
                IoEvents::IN
            }),
            // This is allowed, as we invoke the checking closure.
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            // This is allowed, as we invoke the checking closure.
            //
            // Reusing the cached state is NOT allowed as we've been notified above.
            IoEvents::OUT,
        );
    }
}
