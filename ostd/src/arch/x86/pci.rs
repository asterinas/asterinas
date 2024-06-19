// SPDX-License-Identifier: MPL-2.0

//! PCI bus io port

use super::device::io_port::{IoPort, ReadWriteAccess, WriteOnlyAccess};

pub static PCI_ADDRESS_PORT: IoPort<u32, WriteOnlyAccess> = unsafe { IoPort::new(0x0CF8) };
pub static PCI_DATA_PORT: IoPort<u32, ReadWriteAccess> = unsafe { IoPort::new(0x0CFC) };
