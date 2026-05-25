// SPDX-License-Identifier: MPL-2.0

//! IRQ remapping support (stub).

pub(crate) struct IrqRemapping {
    _private: (),
}

impl IrqRemapping {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    /// Initializes the remapping entry for the specific IRQ number.
    pub(crate) fn init(&self, _irq_num: u8) {}

    /// Gets the remapping index of the IRQ line.
    pub(crate) fn remapping_index(&self) -> Option<u16> {
        None
    }
}
