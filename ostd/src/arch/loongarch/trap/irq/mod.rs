// SPDX-License-Identifier: MPL-2.0

//! Reference: <https://godones.github.io/rCoreloongArch/interrupt.html>

use loongArch64::register::ecfg::LineBasedInterrupt;

use crate::arch::{self, trap::irq::eiointc::Eiointc};

pub mod eiointc;

pub fn init() {
    // FIXME: Support SMP in LoongArch
    Eiointc::init(1);
    for i in arch::irq::IRQ_NUM_MIN..=arch::irq::IRQ_NUM_MAX {
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

pub fn claim() -> Option<u8> {
    Eiointc::claim()
}

pub fn complete(irq: u8) {
    Eiointc::complete(irq);
}
