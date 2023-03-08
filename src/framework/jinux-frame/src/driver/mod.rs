//! Driver for APIC, PIC, PIT etc.
//! This module should inaccessible by other crate such as std, virtio etc.
//!

mod acpi;
mod ioapic;
mod pic;
pub(crate) mod rtc;
mod timer;
mod xapic;

pub(crate) use self::pic::ack as pic_ack;
pub(crate) use self::pic::allocate_irq as pic_allocate_irq;
pub(crate) use self::xapic::ack as xapic_ack;
use log::info;
pub(crate) use timer::{add_timeout_list, TimerCallback, TICK};

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
    rtc::init();
}
