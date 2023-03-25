//! Driver for APIC, PIC, PIT etc.
//!

mod acpi;
mod ioapic;
mod pic;
mod timer;
mod xapic;

pub(crate) use self::acpi::ACPI_TABLES;
pub(crate) use self::pic::ack as pic_ack;
pub(crate) use self::pic::allocate_irq as pic_allocate_irq;
pub(crate) use self::xapic::ack as xapic_ack;
pub(crate) use timer::{add_timeout_list, TimerCallback, TICK};

use log::info;

pub(crate) fn init() {
    acpi::init();
    if xapic::has_apic() {
        ioapic::init();
        xapic::init();
    } else {
        info!("No apic exists, using pic instead");
        unsafe {
            pic::enable();
        }
    }
    timer::init();
    pic::init();
}
