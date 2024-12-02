// SPDX-License-Identifier: MPL-2.0

//! PCI bus io port

use super::device::io_port::{IoPort, ReadWriteAccess, WriteOnlyAccess};

pub static PCI_ADDRESS_PORT: IoPort<WriteOnlyAccess, u32> = unsafe { IoPort::new(0x0CF8) };
pub static PCI_DATA_PORT: IoPort<ReadWriteAccess, u32> = unsafe { IoPort::new(0x0CFC) };
