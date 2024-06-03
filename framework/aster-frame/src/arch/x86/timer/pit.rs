// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! The Programmable Interval Timer (PIT) chip (Intel 8253/8254) basically consists of an oscillator,
//! a prescaler and 3 independent frequency dividers. Each frequency divider has an output, which is
//! used to allow the timer to control external circuitry (for example, IRQ 0).
//!
//! Reference: <https://wiki.osdev.org/Programmable_Interval_Timer>
//!

use crate::{
    arch::{
        kernel::IO_APIC,
        timer::TIMER_FREQ,
        x86::device::io_port::{IoPort, WriteOnlyAccess},
    },
    trap::IrqLine,
};

/// PIT Operating Mode.
///
/// Usually, only the rate generator, which is used to determine the base frequency of other timers
/// (e.g. APIC Timer), and the Square wave generator, which is used to generate interrupts directly, are used.
///
/// Note that if IOAPIC is used to manage interrupts and square wave mode is enabled, the frequency at which
/// clock interrupts are generated is `Frequency/2`.
#[repr(u8)]
pub enum OperatingMode {
    /// Triggers an interrupt (only on channel 0) when the counter is terminated (1 -> 0).
    /// The data port needs to be reset before the next interrupt.
    /// ```text,ignore
    ///            software reload counter
    ///                      ⬇
    ///               +------+             +----
    ///               |      |             |
    /// --------------+      +-------------+
    /// ⬆             ⬆                    ⬆
    /// init()   counter 1 -> 0         counter 1 -> 0
    /// ```
    InterruptOnTerminalCount = 0b000,
    /// This mode is similar to `InterruptOnTerminalCount` mode, however counting doesn't start until
    /// a rising edge of the gate input is detected. For this reason it is not usable for PIT channels
    /// 0 or 1(where the gate input can't be changed).
    OneShotHardwareRetriggerable = 0b001,
    /// Rate generator, which produces a pulse at a fixed frequency.
    /// ```text,ignore
    /// init()   counter 2 -> 1    counter 2 -> 1
    /// ⬇             ⬇                ⬇
    /// --------------+  +-------------+
    ///               |  |             |
    ///               +--+             +--
    ///                  ⬆
    ///     counter 1 -> 0, auto reload counter
    /// ```
    RateGenerator = 0b010,
    /// In this mode, the current count is **decremented twice** on each falling edge of the input signal.
    /// The output will change state and then set to reload value.
    /// ```text,ignore
    /// init()  auto reload counter
    /// ⬇             ⬇
    /// --------------+              +--------------
    ///               |              |
    ///               +--------------+
    ///                              ⬆
    ///                       auto reload counter
    /// ```
    SquareWaveGenerator = 0b011,
    /// Similar to a Rate generator, but requires a software reset to start counting.
    /// ```text,ignore
    /// init()   counter: 1  software reload counter
    /// ⬇             ⬇              ⬇
    /// --------------+ +---------------------------+ +--
    ///               | |                           | |
    ///               +-+                           +-+
    ///                 ⬆
    ///              counter: 0
    /// ```
    SoftwareTriggeredStrobe = 0b100,
    /// This mode is similar to `SoftwareTriggeredStrobe` mode, except that it waits for the rising
    /// edge of the gate input to trigger (or re-trigger) the delay period (like `OneShotHardwareRetriggerable`
    /// mode).
    HardwareTriggeredStrobe = 0b101,
    // 0b110 -> Rate Generator
    // 0b111 -> Square Wave Generator
}

/// This bits tell the PIT what access mode is used for the selected channel.
#[repr(u8)]
enum AccessMode {
    /// When this command is sent, the current count is copied into latch register which can be read
    /// through the data port corresponding to the selected channel (I/O ports 0x40 to 0x42).
    LatchCountValueCommand = 0b00,
    /// Only the lowest 8 bits of the count value are used in this mode.
    LowByteOnly = 0b01,
    /// Only the highest 8 bits of the count value are used in this mode.
    HighByteOnly = 0b10,
    /// 16 bits are used in this mode. User should sent the lowest 8 bits followed by the highest 8 bits
    /// to the same data port.
    LowAndHighByte = 0b11,
}

/// Used to select the configured channel in the `MODE_COMMAND_PORT` of the PIT.
#[repr(u8)]
enum Channel {
    /// Channel 0. For more details, check `CHANNEL0_PORT` static variable
    Channel0 = 0b00,
    /// Channel 1. For more details, check `CHANNEL1_PORT` static variable
    Channel1 = 0b01,
    /// Channel 2. For more details, check `CHANNEL2_PORT` static variable
    Channel2 = 0b10,
    /// The read back command is a special command sent to the mode/command register.
    /// The register uses the following format if set to read back command:
    /// ```text
    /// Bits         Usage
    /// 7 and 6      Must be set for the read back command
    /// 5            Latch count flag (0 = latch count, 1 = don't latch count)
    /// 4            Latch status flag (0 = latch status, 1 = don't latch status)
    /// 3            Read back timer channel 2 (1 = yes, 0 = no)
    /// 2            Read back timer channel 1 (1 = yes, 0 = no)
    /// 1            Read back timer channel 0 (1 = yes, 0 = no)
    /// 0            Reserved
    /// ```
    /// Bits 1 to 3 of the read back command select which PIT channels are affected,
    /// and allow multiple channels to be selected at the same time.
    ///
    /// If bit 5 is clear, then any/all PIT channels selected with bits 1 to 3 will
    /// have their current count copied into their latch register.
    ///
    /// If bit 4 is clear, then for any/all PIT channels selected with bits 1 to 3,
    /// the next read of the corresponding data port will return a status byte.
    ///
    /// Ref: https://wiki.osdev.org/Programmable_Interval_Timer#Read_Back_Command
    ReadBackCommand = 0b11,
}

/// The output from PIT channel 0 is connected to the PIC chip and generate "IRQ 0".
/// If connected to PIC, the IRQ0 will generate by the **rising edge** of the output voltage.
static CHANNEL0_PORT: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x40) };

/// The output from PIT channel 1 was once used for refreshing the DRAM or RAM so that
/// the capacitors don't forget their state.
///
/// On later machines, the DRAM refresh is done with dedicated hardware and this channel
/// is no longer used.
#[allow(unused)]
static CHANNEL1_PORT: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x41) };

/// The output from PIT channel 2 is connected to the PC speaker, so the frequency of the
/// output determines the frequency of the sound produced by the speaker. For more information,
/// check https://wiki.osdev.org/PC_Speaker.
#[allow(unused)]
static CHANNEL2_PORT: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x42) };

/// PIT command port.
/// ```text
/// Bits         Usage
/// 6 and 7      channel
/// 4 and 5      Access mode
/// 1 to 3       Operating mode
/// 0            BCD/Binary mode: 0 = 16-bit binary, 1 = four-digit BCD
/// ```
static MODE_COMMAND_PORT: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x43) };
const TIMER_RATE: u32 = 1193182;

pub(crate) fn init(operating_mode: OperatingMode) {
    // Set PIT mode
    // Bit 0 is BCD/binary mode, which is always set to binary mode(value: 0)
    MODE_COMMAND_PORT.write(
        ((operating_mode as u8) << 1)
            | (AccessMode::LowAndHighByte as u8) << 4
            | (Channel::Channel0 as u8) << 6,
    );

    // Set timer frequency
    const CYCLE: u32 = TIMER_RATE / TIMER_FREQ as u32;
    CHANNEL0_PORT.write((CYCLE & 0xFF) as _);
    CHANNEL0_PORT.write((CYCLE >> 8) as _);
}

/// Enable the IOAPIC line that connected to PIC
pub(crate) fn enable_ioapic_line(irq: IrqLine) {
    let mut io_apic = IO_APIC.get().unwrap().first().unwrap().lock();
    debug_assert_eq!(io_apic.interrupt_base(), 0);
    io_apic.enable(2, irq.clone()).unwrap();
}

/// Disable the IOAPIC line that connected to PIC
pub(crate) fn disable_ioapic_line() {
    let mut io_apic = IO_APIC.get().unwrap().first().unwrap().lock();
    debug_assert_eq!(io_apic.interrupt_base(), 0);
    io_apic.disable(2).unwrap();
}
