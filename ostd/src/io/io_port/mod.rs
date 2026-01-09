// SPDX-License-Identifier: MPL-2.0

//! I/O port and its allocator that allocates port I/O (PIO) to device drivers.

use crate::arch::device::io_port::{IoPortReadAccess, IoPortWriteAccess, PortRead, PortWrite};
mod allocator;

use core::marker::PhantomData;

pub(super) use self::allocator::init;
use crate::{Error, prelude::*};

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
#[derive(Debug)]
pub struct IoPort<T, A> {
    port: u16,
    is_overlapping: bool,
    value_marker: PhantomData<T>,
    access_marker: PhantomData<A>,
}

impl<T, A> IoPort<T, A> {
    /// Acquires an `IoPort` instance for the given range.
    ///
    /// This method will mark all ports in the PIO range as occupied.
    pub fn acquire(port: u16) -> Result<IoPort<T, A>> {
        allocator::IO_PORT_ALLOCATOR
            .get()
            .unwrap()
            .acquire(port, false)
            .ok_or(Error::AccessDenied)
    }

    /// Acquires an `IoPort` instance that may overlap with other `IoPort`s.
    ///
    /// This method will only mark the first port in the PIO range as occupied.
    pub fn acquire_overlapping(port: u16) -> Result<IoPort<T, A>> {
        allocator::IO_PORT_ALLOCATOR
            .get()
            .unwrap()
            .acquire(port, true)
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

    /// Creates an I/O port.
    ///
    /// # Safety
    ///
    /// Reading from or writing to the I/O port may have side effects. Those side effects must not
    /// cause soundness problems (e.g., they must not corrupt the kernel memory).
    pub(crate) const unsafe fn new(port: u16) -> Self {
        // SAFETY: The safety is upheld by the caller.
        unsafe { Self::new_overlapping(port, false) }
    }

    /// Creates an I/O port.
    ///
    /// See [`IoPortAllocator::acquire`] for an explanation of the `is_overlapping` argument.
    ///
    /// [`IoPortAllocator::acquire`]: allocator::IoPortAllocator::acquire
    ///
    /// # Safety
    ///
    /// Reading from or writing to the I/O port may have side effects. Those side effects must not
    /// cause soundness problems (e.g., they must not corrupt the kernel memory).
    const unsafe fn new_overlapping(port: u16, is_overlapping: bool) -> Self {
        Self {
            port,
            is_overlapping,
            value_marker: PhantomData,
            access_marker: PhantomData,
        }
    }
}

impl<T: PortRead, A: IoPortReadAccess> IoPort<T, A> {
    /// Reads from the I/O port
    pub fn read(&self) -> T {
        unsafe { PortRead::read_from_port(self.port) }
    }
}

impl<T: PortWrite, A: IoPortWriteAccess> IoPort<T, A> {
    /// Writes to the I/O port
    pub fn write(&self, value: T) {
        unsafe { PortWrite::write_to_port(self.port, value) }
    }
}

impl<T, A> Drop for IoPort<T, A> {
    fn drop(&mut self) {
        let range = if !self.is_overlapping {
            self.port..(self.port + size_of::<T>() as u16)
        } else {
            self.port..(self.port + 1)
        };

        // SAFETY: We have ownership of the PIO region.
        unsafe { allocator::IO_PORT_ALLOCATOR.get().unwrap().recycle(range) };
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
            // SAFETY: This is properly handled in the linker script.
            #[unsafe(link_section = ".sensitive_io_ports")]
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
                // SAFETY: This is properly handled in the linker script.
                #[unsafe(link_section = ".sensitive_io_ports")]
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
