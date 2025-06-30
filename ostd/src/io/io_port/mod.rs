// SPDX-License-Identifier: MPL-2.0

//! I/O port and its allocator that allocates port I/O (PIO) to device drivers.

use crate::arch::device::io_port::{IoPortReadAccess, IoPortWriteAccess, PortRead, PortWrite};
mod allocator;

use core::{marker::PhantomData, mem::size_of};

pub(super) use self::allocator::init;
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

    /// Returns the port number.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the size of the I/O port.
    pub const fn size(&self) -> u16 {
        size_of::<T>() as u16
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

/// Reserves an I/O port range which may refer to the port I/O range used by the
/// system device driver.
///
/// # Example
/// ```
/// reserve_io_port_range!(0x60..0x64);
/// ```
macro_rules! reserve_io_port_range {
    ($range:expr) => {
        crate::const_assert!(
            $range.start < $range.end,
            "I/O port range must be valid (start < end)"
        );

        const _: () = {
            #[used]
            #[link_section = ".sensitive_io_ports"]
            static _RANGE: crate::io::RawIoPortRange = crate::io::RawIoPortRange {
                begin: $range.start,
                end: $range.end,
            };
        };
    };
}

/// Declares one or multiple sensitive I/O ports.
///
/// # Safety
///
/// User must ensures that:
/// - The I/O port is valid and doesn't overlap with other sensitive I/O ports.
/// - The I/O port is used by the target system device driver.
///
/// # Example
/// ```no_run
/// sensitive_io_port! {
///     unsafe {
///         /// Master PIC command port
///         static MASTER_CMD: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x20);
///         /// Master PIC data port
///         static MASTER_DATA: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x21);
///     }
/// }
/// ```
macro_rules! sensitive_io_port {
    (unsafe { $(
        $(#[$meta:meta])*
        $vis:vis static $name:ident: IoPort<$size:ty, $access:ty> = IoPort::new($port:expr);
    )* }) => {
        $(
            $(#[$meta])*
            $vis static $name: IoPort<$size, $access> = {
                #[used]
                #[link_section = ".sensitive_io_ports"]
                static _RESERVED_IO_PORT_RANGE: crate::io::RawIoPortRange = crate::io::RawIoPortRange {
                    begin: $name.port(),
                    end: $name.port() + $name.size(),
                };

            	unsafe {
                     IoPort::new($port)
            	}
            };
        )*
    };
}

pub(crate) use reserve_io_port_range;
pub(crate) use sensitive_io_port;

#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub(crate) struct RawIoPortRange {
    pub(crate) begin: u16,
    pub(crate) end: u16,
}
