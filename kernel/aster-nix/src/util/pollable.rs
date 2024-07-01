use crate::{events::IoEvents, prelude::*, process::signal::Poller};

pub trait Pollable {
    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents;

    fn wait_events<F, R>(&self, mask: IoEvents, mut cond: F) -> Result<R>
    where
        F: FnMut() -> Result<R>,
    {
        self.wait_events_with_state(mask, |_| cond().map_err(|err| (err, ())), ())
            .map_err(|(err, _)| err)
    }

    // TODO: Support timeout
    fn wait_events_with_state<F, R, S>(
        &self,
        mask: IoEvents,
        mut cond: F,
        mut state: S,
    ) -> core::result::Result<R, (Error, S)>
    where
        F: FnMut(S) -> core::result::Result<R, (Error, S)>,
    {
        let poller = Poller::new();

        loop {
            match cond(state) {
                Err((err, new_state)) if err.error() == Errno::EAGAIN => state = new_state,
                result => return result,
            };

            let events = self.poll(mask, Some(&poller));
            if !events.is_empty() {
                continue;
            }

            if let Err(err) = poller.wait() {
                return Err((err, state));
            }
        }
    }
}
