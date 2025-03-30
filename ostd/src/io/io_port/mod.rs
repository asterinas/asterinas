// SPDX-License-Identifier: MPL-2.0

//! I/O port and its allocator that allocates port I/O (PIO) to device drivers.

use core::{marker::PhantomData, mem::size_of};

use crate::{
    arch::device::io_port::{IoPortReadAccess, IoPortWriteAccess, PortRead, PortWrite},
    mm::PodOnce,
    prelude::*,
    Error,
};

/// An I/O port, representing a specific address in the I/O address.
///
/// The following code shows and example to read and write u32 value to an I/O port:
///
/// ```rust
/// static PORT: IoPort<ReadWriteAccess> = unsafe { IoPort::new::<u32>(0x12) };
///
/// fn port_value_increase(){
///     PORT.write::<u32>(0, PORT.read::<u32>(0).unwrap() + 1).unwrap();
/// }
/// ```
///
pub struct IoPort<A> {
    base: u16,
    size: u16,
    access_marker: PhantomData<A>,
}

impl<A> IoPort<A> {
    /// Base address of the I/O port.
    pub fn base(&self) -> u16 {
        self.base
    }

    /// The size of the I/O port (in bytes)
    pub fn size(&self) -> u16 {
        self.size
    }

    /// Creates an I/O port.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub(crate) const unsafe fn new<T: PodOnce>(base: u16) -> Self {
        Self {
            base,
            access_marker: PhantomData,
            size: size_of::<T>() as u16,
        }
    }

    /// Creates an I/O port with size.
    ///
    /// Panic if the size is smaller than the size of the value type.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub(crate) const unsafe fn new_with_size(base: u16, size: u16) -> Self {
        assert!(size > size_of::<A>() as u16);
        Self {
            base,
            size,
            access_marker: PhantomData,
        }
    }
}

impl<A: IoPortReadAccess> IoPort<A> {
    /// Reads from I/O port
    #[inline]
    pub fn read<T: PortRead>(&self, offset: u16) -> Result<T> {
        // Check alignment
        if (self.base() + offset) % size_of::<T>() as u16 != 0 {
            return Err(Error::InvalidArgs);
        }
        // Check overflow
        if self.size() < size_of::<T>() as u16 || offset > self.size() - size_of::<T>() as u16 {
            return Err(Error::InvalidArgs);
        }
        // SAFETY: The range of ports accessed is within the scope managed by the IoPort and
        // an out-of-bounds check is performed.
        unsafe { Ok(T::read_from_port(self.base)) }
    }

    /// Reads from I/O port with no offset
    #[inline]
    pub fn read_no_offset<T: PortRead>(&self) -> Result<T> {
        self.read(0)
    }
}

impl<A: IoPortWriteAccess> IoPort<A> {
    /// Writes to I/O port
    #[inline]
    pub fn write<T: PortWrite>(&self, offset: u16, value: T) -> Result<()> {
        // Check alignment
        if (self.base() + offset) % size_of::<T>() as u16 != 0 {
            return Err(Error::InvalidArgs);
        }
        // Check overflow
        if self.size() < size_of::<T>() as u16 || offset > self.size() - size_of::<T>() as u16 {
            return Err(Error::InvalidArgs);
        }
        // SAFETY: The range of ports accessed is within the scope managed by the IoPort and
        // an out-of-bounds check is performed.
        unsafe { T::write_to_port(self.base, value) }
        Ok(())
    }

    /// Writes to I/O port with no offset
    #[inline]
    pub fn write_no_offset<T: PortWrite>(&self, value: T) -> Result<()> {
        self.write(0, value)
    }
}
