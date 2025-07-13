// SPDX-License-Identifier: MPL-2.0

mod eiointc;

use loongArch64::register::ecfg::LineBasedInterrupt;

use crate::arch::{irq, kernel::irq::eiointc::Eiointc};

pub(in crate::arch) fn init() {
    // FIXME: Support SMP in LoongArch
    Eiointc::init(1);
    for i in irq::IRQ_NUM_MIN..=irq::IRQ_NUM_MAX {
        Eiointc::enable(i);
    }
    loongArch64::register::ecfg::set_lie(
        LineBasedInterrupt::HWI0
            | LineBasedInterrupt::HWI1
            | LineBasedInterrupt::HWI2
            | LineBasedInterrupt::HWI3
            | LineBasedInterrupt::HWI4
            | LineBasedInterrupt::HWI5
            | LineBasedInterrupt::HWI6
            | LineBasedInterrupt::HWI7,
    );
}

pub(in crate::arch) fn claim() -> Option<u8> {
    Eiointc::claim()
}

pub(in crate::arch) fn complete(irq: u8) {
    Eiointc::complete(irq);
}
