// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

/// Initializes the serial port.
pub(crate) fn init() {}

/// Sends a byte on the serial port.
///
/// Uses legacy SBI putchar (EID=0x01) directly via inline assembly,
/// rather than going through `sbi_rt::console_write_byte`.  That
/// function enters an infinite ecall loop on OpenSBI 1.5.1 (SBI 2.0)
/// because its binary-interface version check does not match.
pub fn send(data: u8) {
    unsafe {
        core::arch::asm!(
            "li a7, 0x01",
            "ecall",
            in("a0") data as usize,
        );
    }
}
