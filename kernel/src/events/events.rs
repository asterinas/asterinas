// SPDX-License-Identifier: MPL-2.0

/// A trait to represent any events.
///
/// # The unit event
///
/// The unit type `()` can serve as a unit event.
/// It can be used if there is only one kind of event
/// and the event carries no additional information.
pub trait Events: Copy + Clone + Send + Sync + 'static {}

impl Events for () {}

/// A trait to filter events.
///
/// # The no-op event filter
///
/// The unit type `()` can serve as a no-op event filter.
/// It implements `EventsFilter<E>` for any events type `E`,
/// with a `filter` method that always returns `true`.
/// If the `F` type of `Subject<E, F>` is not specified explicitly,
/// then the unit type `()` is chosen as the event filter.
///
/// # Per-object event filter
///
/// Any `Option<F: EventsFilter>` is also an event filter thanks to
/// the blanket implementations the `EventsFilter` trait.
/// By using `Option<F: EventsFilter>`, we can decide, on a per-observer basis,
/// if an observer needs an event filter.
pub trait EventsFilter<E: Events>: Send + Sync + 'static {
    fn filter(&self, event: &E) -> bool;
}

impl<E: Events> EventsFilter<E> for () {
    fn filter(&self, _events: &E) -> bool {
        true
    }
}

impl<E: Events, F: EventsFilter<E>> EventsFilter<E> for Option<F> {
    fn filter(&self, events: &E) -> bool {
        self.as_ref().map_or(true, |f| f.filter(events))
    }
}
