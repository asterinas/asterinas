use crate::{prelude::*, x86_64_util};

/// An I/O port, representing a specific address in the I/O address of x86.
pub struct IoPort {
    addr: u16,
}

impl IoPort {
    /// Create an I/O port.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub unsafe fn new(addr: u16) -> Result<Self> {
        Ok(Self { addr: addr })
    }
}

impl IoPort {
    /// Get the address of this I/O port.
    pub fn addr(&self) -> u16 {
        self.addr
    }

    /// Read a value of `u32`.
    pub fn read_u32(&self) -> u32 {
        x86_64_util::in32(self.addr)
    }

    /// Write a value of `u32`.
    pub fn write_u32(&self, val: u32) {
        x86_64_util::out32(self.addr, val)
    }

    /// Read a value of `u16`.
    pub fn read_u16(&self) -> u16 {
        x86_64_util::in16(self.addr)
    }

    /// Write a value of `u16`.
    pub fn write_u16(&self, val: u16) {
        x86_64_util::out16(self.addr, val)
    }

    /// Read a value of `u8`.
    pub fn read_u8(&self) -> u8 {
        x86_64_util::in8(self.addr)
    }

    /// Write a value of `u8`.
    pub fn write_u8(&self, val: u8) {
        x86_64_util::out8(self.addr, val)
    }
}
