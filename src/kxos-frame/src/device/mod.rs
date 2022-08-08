//! Device-related APIs.

mod io_port;
mod irq;

pub use self::io_port::IoPort;
pub use self::irq::{IrqCallbackHandle, IrqLine};
