//! PCI bus io port

use super::io_port::IoPort;

pub static PCI_ADDRESS_PORT: IoPort<u32> = unsafe { IoPort::new(0x0CF8) };
pub static PCI_DATA_PORT: IoPort<u32> = unsafe { IoPort::new(0x0CFC) };
