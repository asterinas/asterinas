// SPDX-License-Identifier: MPL-2.0

pub mod console;
pub mod cpu;
pub mod irq;
pub mod mm;
pub mod system;

mod sbi;

pub(crate) fn before_all_init() {}

pub(crate) fn after_all_init() {
    irq::init();
}
