// SPDX-License-Identifier: MPL-2.0

//! I/O port and its allocator that allocates port I/O (PIO) to device drivers.

use crate::arch::device::io_port::{IoPortReadAccess, IoPortWriteAccess, PortRead, PortWrite};
mod allocator;

use core::{marker::PhantomData, mem::size_of};

pub(super) use self::allocator::init;
pub(crate) use self::allocator::IoPortAllocatorBuilder;
use crate::{prelude::*, Error};

/// An I/O port, representing a specific address in the I/O address of x86.
///
/// The following code shows and example to read and write u32 value to an I/O port:
///
/// ```rust
/// static PORT: IoPort<u32, ReadWriteAccess> = unsafe { IoPort::new(0x12) };
///
/// fn port_value_increase(){
///     PORT.write(PORT.read() + 1)
/// }
/// ```
///
pub struct IoPort<T, A> {
    port: u16,
    value_marker: PhantomData<T>,
    access_marker: PhantomData<A>,
}

impl<T, A> IoPort<T, A> {
    /// Acquires an `IoPort` instance for the given range.
    pub fn acquire(port: u16) -> Result<IoPort<T, A>> {
        allocator::IO_PORT_ALLOCATOR
            .get()
            .unwrap()
            .acquire(port)
            .ok_or(Error::AccessDenied)
    }

    /// Create an I/O port.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub const unsafe fn new(port: u16) -> Self {
        Self {
            port,
            value_marker: PhantomData,
            access_marker: PhantomData,
        }
    }
}

impl<T: PortRead, A: IoPortReadAccess> IoPort<T, A> {
    /// Reads from the I/O port
    #[inline]
    pub fn read(&self) -> T {
        unsafe { PortRead::read_from_port(self.port) }
    }
}

impl<T: PortWrite, A: IoPortWriteAccess> IoPort<T, A> {
    /// Writes to the I/O port
    #[inline]
    pub fn write(&self, value: T) {
        unsafe { PortWrite::write_to_port(self.port, value) }
    }
}

impl<T, A> Drop for IoPort<T, A> {
    fn drop(&mut self) {
        // SAFETY: The caller have ownership of the PIO region.
        unsafe {
            allocator::IO_PORT_ALLOCATOR
                .get()
                .unwrap()
                .recycle(self.port..(self.port + size_of::<T>() as u16));
        }
    }
}
