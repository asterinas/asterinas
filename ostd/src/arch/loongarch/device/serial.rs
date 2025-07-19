// SPDX-License-Identifier: MPL-2.0

//! A memory-mapped UART.

/// A memory-mapped UART driver for LoongArch.
///
/// Reference: <https://godones.github.io/rCoreloongArch/uart.html>
pub struct Serial {
    base_address: usize,
}

impl Serial {
    /// Creates a new serial driver.
    ///
    /// # Safety
    ///
    /// The base address must be a valid memory address.
    pub const unsafe fn new(base_address: usize) -> Self {
        Self { base_address }
    }

    /// Sends data to the UART.
    pub fn send(&mut self, c: u8) {
        let ptr = self.base_address as *mut u8;
        loop {
            // SAFETY: The safety requirements are upheld by the constructor.
            unsafe {
                let c = ptr.add(5).read_volatile();
                if c & (1 << 5) != 0 {
                    break;
                }
            }
        }
        // SAFETY: The safety requirements are upheld by the constructor.
        unsafe {
            ptr.add(0).write_volatile(c);
        }
    }

    /// Receives data from the UART.
    pub fn recv(&mut self) -> Option<u8> {
        let ptr = self.base_address as *mut u8;
        // SAFETY: The safety requirements are upheld by the constructor.
        unsafe {
            if ptr.add(5).read_volatile() & 1 == 0 {
                // The DR bit is 0, meaning no data
                None
            } else {
                // The DR bit is 1, meaning data!
                Some(ptr.add(0).read_volatile())
            }
        }
    }
}
