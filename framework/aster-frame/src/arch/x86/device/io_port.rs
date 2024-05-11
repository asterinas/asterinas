// SPDX-License-Identifier: MPL-2.0

use core::{marker::PhantomData, mem::size_of};

pub use x86_64::{
    instructions::port::{
        PortReadAccess as IoPortReadAccess, PortWriteAccess as IoPortWriteAccess, ReadOnlyAccess,
        ReadWriteAccess, WriteOnlyAccess,
    },
    structures::port::{PortRead, PortWrite},
};

use crate::{Error, Result};

/// An I/O port, representing a specific address in the I/O address of x86.
///
/// The following code shows and example to read and write u32 value to an I/O port:
///
/// ```rust
/// static PORT: IoPort<ReadWriteAccess, u32> = unsafe { IoPort::new(0x12) };
///
/// fn port_value_increase(){
///     PORT.write(PORT.read() + 1)
/// }
/// ```
///
#[derive(Debug)]
pub struct IoPort<A, T = RunTimeReadWrite> {
    base: u16,
    size: u16,
    value_marker: PhantomData<T>,
    access_marker: PhantomData<A>,
}

impl<T, A> IoPort<T, A> {
    /// Create an I/O port.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub const unsafe fn new(base: u16) -> Self {
        Self {
            base,
            size: size_of::<T>() as u16,
            value_marker: PhantomData,
            access_marker: PhantomData,
        }
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    pub fn size(&self) -> u16 {
        self.size
    }
}

impl<A> IoPort<RunTimeReadWrite, A> {
    /// Create an I/O port with size.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub const unsafe fn new_with_size(base: u16, size: u16) -> Self {
        Self {
            base,
            size,
            value_marker: PhantomData,
            access_marker: PhantomData,
        }
    }
}

impl<A: IoPortReadAccess> IoPort<A> {
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
}

impl<A: IoPortWriteAccess> IoPort<A> {
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
}

impl<A: IoPortReadAccess, T: PortRead> IoPort<A, T> {
    #[inline]
    pub fn read(&self) -> T {
        unsafe { T::read_from_port(self.base) }
    }
}

impl<A: IoPortWriteAccess, T: PortWrite> IoPort<A, T> {
    #[inline]
    pub fn write(&self, value: T) {
        unsafe { T::write_to_port(self.base, value) }
    }
}

/// Port I/O definition reference: https://bochs.sourceforge.io/techspec/PORTS.LST
pub(crate) const IO_PORT_MAX: u16 = u16::MAX;

#[derive(Debug)]
pub struct RunTimeReadWrite {}
