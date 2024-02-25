// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

pub use x86_64::{
    instructions::port::{
        PortReadAccess as IoPortReadAccess, PortWriteAccess as IoPortWriteAccess, ReadOnlyAccess,
        ReadWriteAccess, WriteOnlyAccess,
    },
    structures::port::{PortRead, PortWrite},
};

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
    #[inline]
    pub fn read(&self) -> T {
        unsafe { PortRead::read_from_port(self.port) }
    }
}

impl<T: PortWrite, A: IoPortWriteAccess> IoPort<T, A> {
    #[inline]
    pub fn write(&self, value: T) {
        unsafe { PortWrite::write_to_port(self.port, value) }
    }
}
