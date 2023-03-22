//! PCI bus io port

use super::io_port::IoPort;
use super::io_port::{ReadWriteAccess, WriteOnlyAccess};

pub static PCI_ADDRESS_PORT: IoPort<u32, WriteOnlyAccess> = unsafe { IoPort::new(0x0CF8) };
pub static PCI_DATA_PORT: IoPort<u32, ReadWriteAccess> = unsafe { IoPort::new(0x0CFC) };
