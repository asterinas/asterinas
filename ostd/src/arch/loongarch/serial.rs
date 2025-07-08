// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use spin::Once;

use crate::arch::{boot::DEVICE_TREE, mm::paddr_to_daddr};

bitflags::bitflags! {
    struct LineStatusRegisterFlags: u8 {
        const DR    = 1 << 0;
        const OE    = 1 << 1;
        const PE    = 1 << 2;
        const FE    = 1 << 3;
        const BI    = 1 << 4;
        const TFE   = 1 << 5;
        const TE    = 1 << 6;
        const ERROR = 1 << 7;
    }
}

/// A memory-mapped UART driver for LoongArch.
///
/// Reference: <https://loongson.github.io/LoongArch-Documentation/Loongson-3A5000-usermanual-EN.html#uart-controller>
struct Serial {
    base_address: *mut u8,
}

impl Serial {
    const DATA_TRANSPORT_REGISTER_OFFSET: usize = 0;
    const LINE_STATUS_REGISTER_OFFSET: usize = 5;

    /// Creates a serial driver.
    ///
    /// # Safety
    ///
    /// The base address must be a valid UART base address.
    const unsafe fn new(base_address: *mut u8) -> Self {
        Self { base_address }
    }

    /// Sends data to the UART.
    fn send(&self, c: u8) {
        while !self
            .line_status_register_flags()
            .contains(LineStatusRegisterFlags::TFE)
        {
            core::hint::spin_loop();
        }
        // SAFETY: The safety requirements are upheld by the constructor.
        unsafe {
            self.base_address
                .add(Self::DATA_TRANSPORT_REGISTER_OFFSET)
                .write_volatile(c);
        }
    }

    /// Receives data from the UART.
    fn recv(&self) -> Option<u8> {
        if self
            .line_status_register_flags()
            .contains(LineStatusRegisterFlags::DR)
        {
            // SAFETY: The safety requirements are upheld by the constructor.
            Some(unsafe {
                self.base_address
                    .add(Self::DATA_TRANSPORT_REGISTER_OFFSET)
                    .read_volatile()
            })
        } else {
            None
        }
    }

    fn line_status_register_flags(&self) -> LineStatusRegisterFlags {
        // SAFETY: The safety requirements are upheld by the constructor.
        let c = unsafe {
            self.base_address
                .add(Self::LINE_STATUS_REGISTER_OFFSET)
                .read_volatile()
        };
        LineStatusRegisterFlags::from_bits_truncate(c)
    }
}

// SAFETY: For correctness purposes, the UART registers should not be accessed concurrently.
// However, doing so will not cause memory safety violations.
unsafe impl Send for Serial {}
unsafe impl Sync for Serial {}

/// The console UART.
static CONSOLE_COM1: Once<Serial> = Once::new();

/// Initializes the serial port.
pub(crate) fn init() {
    let Some(base_address) = lookup_uart_base_address() else {
        return;
    };
    // SAFETY: It is safe because the UART address is acquired from the device tree,
    // and be mapped in DMW2.
    CONSOLE_COM1.call_once(|| unsafe { Serial::new(paddr_to_daddr(base_address) as *mut u8) });
}

// FIXME: We should reserve the address region in `io_mem_allocator`.
fn lookup_uart_base_address() -> Option<usize> {
    let device_tree = DEVICE_TREE.get().unwrap();
    let stdout_path = device_tree
        .find_node("/chosen")?
        .property("stdout-path")?
        .as_str()?;
    let stdout = device_tree.find_node(stdout_path)?;
    if stdout.compatible()?.all().any(|c| c == "ns16550a") {
        Some(stdout.reg()?.next()?.starting_address as usize)
    } else {
        None
    }
}

/// Sends a byte on the serial port.
pub(crate) fn send(data: u8) {
    // Note: It is the caller's responsibility to acquire the correct lock and ensure sequential
    // access to the UART registers.
    let Some(uart) = CONSOLE_COM1.get() else {
        return;
    };
    match data {
        b'\n' => {
            uart.send(b'\r');
            uart.send(b'\n');
        }
        c => uart.send(c),
    }
}
