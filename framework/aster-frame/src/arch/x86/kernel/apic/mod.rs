// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use bit_field::BitField;
use log::info;
use spin::Once;

use crate::lazy_cpu_local;

pub mod ioapic;
pub mod x2apic;
pub mod xapic;

lazy_cpu_local! {
    pub static APIC_INSTANCE: Box<dyn Apic + 'static> = {
        match APIC_TYPE.get().unwrap() {
            ApicType::X2Apic => {
                let mut x2apic = x2apic::X2Apic::new().unwrap();
                x2apic.enable();
                let version = x2apic.version();
                info!(
                    "x2APIC ID:{:x}, Version:{:x}, Max LVT:{:x}",
                    x2apic.id(),
                    version & 0xff,
                    (version >> 16) & 0xff
                );
                Box::new(x2apic)
            }
            ApicType::XApic => {
                let mut xapic = xapic::XApic::new().unwrap();
                xapic.enable();
                let version = xapic.version();
                info!(
                    "xAPIC ID:{:x}, Version:{:x}, Max LVT:{:x}",
                    xapic.id(),
                    version & 0xff,
                    (version >> 16) & 0xff
                );
                Box::new(xapic)
            }
        }
    };
}

pub(crate) static APIC_TYPE: Once<ApicType> = Once::new();

pub trait Apic: ApicTimer + Sync + Send {
    fn id(&self) -> u32;

    fn version(&self) -> u32;

    /// End of Interrupt, this function will inform APIC that this interrupt has been processed.
    fn eoi(&mut self);

    /// Send a generic IPI
    unsafe fn send_ipi(&mut self, icr: Icr);
}

pub trait ApicTimer: Sync + Send {
    /// Set the initial timer count, the APIC timer will count down from this value.
    fn set_timer_init_count(&mut self, value: u64);

    /// Get the current count of the timer.
    /// The interval can be expressed by the expression: `init_count` - `current_count`.
    fn timer_current_count(&self) -> u64;

    /// Set the timer register in the APIC.
    /// Bit 0-7:   The interrupt vector of timer interrupt.
    /// Bit 12:    Delivery Status, 0 for Idle, 1 for Send Pending.
    /// Bit 16:    Mask bit.
    /// Bit 17-18: Timer Mode, 0 for One-shot, 1 for Periodic, 2 for TSC-Deadline.
    fn set_lvt_timer(&mut self, value: u64);

    /// Set timer divide config register.
    fn set_timer_div_config(&mut self, div_config: DivideConfig);
}

pub(crate) enum ApicType {
    XApic,
    X2Apic,
}

/// The inter-processor interrupt control register.
///
/// ICR is a 64-bit local APIC register that allows software running on the
/// porcessor to specify and send IPIs to other porcessors in the system.
/// To send an IPI, software must set up the ICR to indicate the type of IPI
/// message to be sent and the destination processor or processors. (All fields
/// of the ICR are read-write by software with the exception of the delivery
/// status field, which is read-only.)
///
/// The act of writing to the low doubleword of the ICR causes the IPI to be
/// sent. Therefore, in xapic mode, high doubleword of the ICR needs to be written
/// first and then the low doubleword to ensure the correct interrupt is sent.
///
/// The ICR consists of the following fields:
/// - **Bit 0-7**   Vector                  :The vector number of the interrupt being sent.
/// - **Bit 8-10**  Delivery Mode           :Specifies the type of IPI to be sent.
/// - **Bit 11**    Destination Mode        :Selects either physical or logical destination mode.
/// - **Bit 12**    Delivery Status(RO)     :Indicates the IPI delivery status.
/// - **Bit 13**    Reserved
/// - **Bit 14**    Level                   :Only set 1 for the INIT level de-assert delivery mode.
/// - **Bit 15**    Trigger Mode            :Selects level or edge trigger mode.
/// - **Bit 16-17** Reserved
/// - **Bit 18-19** Destination Shorthand   :Indicates destination set.
/// - **Bit 20-55** Reserved
/// - **Bit 56-63** Destination Field       :Specifies the target processor or processors.
pub struct Icr(u64);

impl Icr {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        destination: ApicId,
        destination_shorthand: DestinationShorthand,
        trigger_mode: TriggerMode,
        level: Level,
        delivery_status: DeliveryStatus,
        destination_mode: DestinationMode,
        delivery_mode: DeliveryMode,
        vector: u8,
    ) -> Self {
        let dest = match destination {
            ApicId::XApic(d) => (d as u64) << 56,
            ApicId::X2Apic(d) => (d as u64) << 32,
        };
        Icr(dest
            | (destination_shorthand as u64) << 18
            | (trigger_mode as u64) << 15
            | (level as u64) << 14
            | (delivery_status as u64) << 12
            | (destination_mode as u64) << 11
            | (delivery_mode as u64) << 8
            | (vector as u64))
    }

    /// Returns the lower 32 bits of the ICR.
    pub fn lower(&self) -> u32 {
        self.0 as u32
    }

    /// Returns the higher 32 bits of the ICR.
    pub fn upper(&self) -> u32 {
        (self.0 >> 32) as u32
    }
}

/// The core identifier. ApicId can be divided into Physical ApicId and Logical ApicId.
/// The Physical ApicId is the value read from the LAPIC ID Register, while the Logical ApicId has different
/// encoding modes in XApic and X2Apic.
pub enum ApicId {
    XApic(u8),
    X2Apic(u32),
}

impl ApicId {
    /// Returns the logical x2apic ID.
    ///
    /// In x2APIC mode, the 32-bit logical x2APIC ID, which can be read from
    /// LDR, is derived from the 32-bit local x2APIC ID:
    /// Logical x2APIC ID = [(x2APIC ID[19:4] << 16) | (1 << x2APIC ID[3:0])]
    pub fn x2apic_logical_id(&self) -> u32 {
        self.x2apic_logical_cluster_id() << 16 | 1 << self.x2apic_logical_field_id()
    }

    /// Returns the logical x2apic cluster ID.
    ///
    /// Logical cluster ID = x2APIC ID[19:4]
    pub fn x2apic_logical_cluster_id(&self) -> u32 {
        let apic_id = match *self {
            ApicId::XApic(id) => id as u32,
            ApicId::X2Apic(id) => id,
        };
        apic_id.get_bits(4..=19)
    }

    /// Returns the logical x2apic field ID.
    ///
    /// Specifically, the 16-bit logical ID sub-field is derived by the lowest
    /// 4 bits of the x2APIC ID, i.e.,
    /// Logical field ID = x2APIC ID[3:0].
    pub fn x2apic_logical_field_id(&self) -> u32 {
        let apic_id = match *self {
            ApicId::XApic(id) => id as u32,
            ApicId::X2Apic(id) => id,
        };
        apic_id.get_bits(0..=3)
    }
}

impl From<u32> for ApicId {
    fn from(value: u32) -> Self {
        match APIC_TYPE.get().unwrap() {
            ApicType::XApic => ApicId::XApic(value as u8),
            ApicType::X2Apic => ApicId::X2Apic(value),
        }
    }
}

/// Indicates whether a shorthand notation is used to specify the destination of
/// the interrupt and, if so, which shorthand is used. Destination shorthands are
/// used in place of the 8-bit destination field, and can be sent by software
/// using a single write to the low doubleword of the ICR.
///
/// Shorthands are defined for the following cases: software self interrupt, IPIs
/// to all processors in the system including the sender, IPIs to all processors
/// in the system excluding the sender.
#[repr(u64)]
pub enum DestinationShorthand {
    NoShorthand = 0b00,
    MySelf = 0b01,
    AllIncludingSelf = 0b10,
    AllExcludingSelf = 0b11,
}

#[repr(u64)]
pub enum TriggerMode {
    Egde = 0,
    Level = 1,
}

#[repr(u64)]
pub enum Level {
    Deassert = 0,
    Assert = 1,
}

/// Indicates the IPI delivery status (read only), as follows:
/// **0 (Idle)**            Indicates that this local APIC has completed sending any previous IPIs.
/// **1 (Send Pending)**    Indicates that this local APIC has not completed sending the last IPI.
#[repr(u64)]
pub enum DeliveryStatus {
    Idle = 0,
    SendPending = 1,
}

#[repr(u64)]
pub enum DestinationMode {
    Physical = 0,
    Logical = 1,
}

#[repr(u64)]
pub enum DeliveryMode {
    /// Delivers the interrupt specified in the vector field to the target processor or processors.
    Fixed = 0b000,
    /// Same as fixed mode, except that the interrupt is delivered to the processor executing at
    /// the lowest priority among the set of processors specified in the destination field. The
    /// ability for a processor to send a lowest priority IPI is model specific and should be
    /// avoided by BIOS and operating system software.
    LowestPriority = 0b001,
    /// Non-Maskable Interrupt
    Smi = 0b010,
    _Reserved = 0b011,
    /// System Management Interrupt
    Nmi = 0b100,
    /// Delivers an INIT request to the target processor or processors, which causes them to
    /// perform an initialization.
    Init = 0b101,
    /// Start-up Interrupt
    StrartUp = 0b110,
}

#[derive(Debug)]
pub enum ApicInitError {
    /// No x2APIC or xAPIC found.
    NoApic,
}

#[derive(Debug)]
#[repr(u32)]
pub enum DivideConfig {
    Divide1 = 0b1011,
    Divide2 = 0b0000,
    Divide4 = 0b0001,
    Divide8 = 0b0010,
    Divide16 = 0b0011,
    Divide32 = 0b1000,
    Divide64 = 0b1001,
    Divide128 = 0b1010,
}

pub fn init() -> Result<(), ApicInitError> {
    crate::arch::x86::kernel::pic::disable_temp();
    if x2apic::X2Apic::has_x2apic() {
        APIC_TYPE.call_once(|| ApicType::X2Apic);
        Ok(())
    } else if xapic::XApic::has_xapic() {
        APIC_TYPE.call_once(|| ApicType::XApic);
        Ok(())
    } else {
        log::warn!("Not found x2APIC or xAPIC");
        Err(ApicInitError::NoApic)
    }
}
