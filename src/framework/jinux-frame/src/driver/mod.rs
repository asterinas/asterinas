//! Driver for APIC, PIC, PIT etc.
//! This module should inaccessible by other crate such as std, virtio etc.
//!

pub mod acpi;
pub mod apic;
pub mod ioapic;
pub mod pic;
pub mod timer;
pub mod rtc;

pub use apic::ack;
pub use timer::TimerCallback;
pub(crate) use timer::{add_timeout_list, TICK};

use crate::info;

pub(crate) fn init(rsdp: Option<u64>) {
    acpi::init(rsdp.unwrap());
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
