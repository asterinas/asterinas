use alloc::vec::Vec;
use crate::InputEvent;
use core::fmt::Debug;

pub trait InputHandler: Send + Sync + Debug {
    /// Returns the event types the handler can process.
    fn supported_event_types(&self) -> Vec<u16>;

    /// Processes the given event.
    fn handle_event(&self, event: InputEvent) -> Result<(), core::convert::Infallible>;
}