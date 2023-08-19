use crate::sync::Mutex;
use alloc::boxed::Box;
use alloc::sync::Arc;
use log::info;
use spin::Once;

pub mod ioapic;
pub mod x2apic;
pub mod xapic;

pub static APIC_INSTANCE: Once<Arc<Mutex<Box<dyn Apic + 'static>>>> = Once::new();

pub trait Apic: ApicTimer + Sync + Send {
    fn id(&self) -> u32;

    fn version(&self) -> u32;

    /// End of Interrupt, this function will inform APIC that this interrupt has been processed.
    fn eoi(&mut self);
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
    if let Some(mut x2apic) = x2apic::X2Apic::new() {
        x2apic.enable();
        let version = x2apic.version();
        info!(
            "x2APIC ID:{:x}, Version:{:x}, Max LVT:{:x}",
            x2apic.id(),
            version & 0xff,
            (version >> 16) & 0xff
        );
        APIC_INSTANCE.call_once(|| Arc::new(Mutex::new(Box::new(x2apic))));
        Ok(())
    } else if let Some(mut xapic) = xapic::XApic::new() {
        xapic.enable();
        let version = xapic.version();
        info!(
            "xAPIC ID:{:x}, Version:{:x}, Max LVT:{:x}",
            xapic.id(),
            version & 0xff,
            (version >> 16) & 0xff
        );
        APIC_INSTANCE.call_once(|| Arc::new(Mutex::new(Box::new(xapic))));
        Ok(())
    } else {
        log::warn!("Not found x2APIC or xAPIC");
        Err(ApicInitError::NoApic)
    }
}
