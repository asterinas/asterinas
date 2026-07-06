// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

/// Initializes the serial port.
pub(crate) fn init() {}

/// Sends a byte on the serial port.
pub fn send(data: u8) {
    // Use legacy SBI putchar (EID=0x01) instead of sbi-rt's binary
    // interface (sbi_rt::console_write_byte).  sbi-rt 0.0.3 triggers
    // an infinite ecall loop when the SBI spec version check fails
    // (OpenSBI v2.0 vs sbi-rt's expected v1.0 binary format).
    unsafe {
        core::arch::asm!(
            "li a7, 0x01",
            "ecall",
            in("a0") data as usize,
        );
    }
}
