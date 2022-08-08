use core::marker::PhantomData;

use crate::prelude::*;

/// An I/O port, representing a specific address in the I/O address of x86.
pub struct IoPort<T> {
    addr: u32,
    _phantom: PhantomData<T>,
}

impl<T> IoPort<T> {
    /// Create an I/O port.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub unsafe fn new(addr: u32) -> Result<Self> {
        todo!()
    }
}

impl IoPort<u32> {
    /// Get the address of this I/O port.
    pub fn addr(&self) -> u32 {
        todo!()
    }

    /// Read a value of `u32`.
    pub fn read_u32(&self) -> u32 {
        todo!()
    }

    /// Write a value of `u32`.
    pub fn write_u32(&self, val: u32) {
        todo!()
    }
}
