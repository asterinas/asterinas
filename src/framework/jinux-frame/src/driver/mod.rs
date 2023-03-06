//! Driver for APIC, PIC, PIT etc.
//! This module should inaccessible by other crate such as std, virtio etc.
//!

pub mod acpi;
pub mod apic;
pub mod ioapic;
pub mod pic;
pub mod rtc;
pub mod timer;

pub use apic::ack;
use log::info;
pub use timer::TimerCallback;
pub(crate) use timer::{add_timeout_list, TICK};

pub(crate) fn init() {
    acpi::init();
    timer::init();
    if apic::has_apic() {
        ioapic::init();
        apic::init();
    } else {
        info!("No apic exists, using pic instead");
        unsafe {
            pic::enable();
        }
    }
    pic::init();
    rtc::init();
}
