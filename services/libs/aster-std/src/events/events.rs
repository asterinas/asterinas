/// A trait to represent any events.
///
/// # The unit event
///
/// The unit type `()` can serve as a unit event.
/// It can be used if there is only one kind of event
/// and the event carries no additional information.
pub trait Events: Copy + Clone + Send + Sync + 'static {}

impl Events for () {}

/// A trait to select events.
///
/// # The no-op event selector
///
/// The unit type `()` can serve as a no-op event selector.
/// It implements `EventsSelector<E>` for any events type `E`,
/// with a `select` method that always returns `true`.
/// If the `S` type of `Subject<E, S>` is not specified explicitly,
/// then the unit type `()` is chosen as the event selector.
///
/// # Per-object event selector
///
/// Any `Option<S: EventsSelector>` is also an event selector thanks to
/// the blanket implementations the `EventsSelector` trait.
/// By using `Option<S: EventsSelector>`, we can decide, on a per-observer basis,
/// if an observer needs an event selector.
pub trait EventsSelector<E: Events>: Send + Sync + 'static {
    fn select(&self, event: &E) -> bool;
}

impl<E: Events> EventsSelector<E> for () {
    fn select(&self, _events: &E) -> bool {
        true
    }
}

impl<E: Events, S: EventsSelector<E>> EventsSelector<E> for Option<S> {
    fn select(&self, events: &E) -> bool {
        self.as_ref().map_or(true, |f| f.select(events))
    }
}
