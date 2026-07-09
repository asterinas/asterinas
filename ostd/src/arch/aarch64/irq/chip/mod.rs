// SPDX-License-Identifier: MPL-2.0

//! A driver for the ARM Generic Interrupt Controller, version 2 (GICv2).
//!
//! The QEMU `virt` machine (with `gic-version=2`) places the distributor at
//! physical `0x0800_0000` and the CPU interface at `0x0801_0000`. Both are
//! reached through the kernel's linear mapping.
//!
//! TODO: Probe the base addresses and interrupt map from the device tree, and
//! add GICv3 support.

use spin::Once;

use crate::mm::paddr_to_vaddr;

/// Physical base of the GICv2 distributor on QEMU `virt`.
const GICD_BASE_PADDR: usize = 0x0800_0000;
/// Physical base of the GICv2 CPU interface on QEMU `virt`.
const GICC_BASE_PADDR: usize = 0x0801_0000;

// Distributor registers.
const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER: usize = 0x100;
const GICD_ICENABLER: usize = 0x180;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;
const GICD_ICFGR: usize = 0xc00;
const GICD_SGIR: usize = 0xf00;

// CPU interface registers.
const GICC_CTLR: usize = 0x00;
const GICC_PMR: usize = 0x04;
const GICC_IAR: usize = 0x0c;
const GICC_EOIR: usize = 0x10;

/// Spurious interrupt ID returned by the CPU interface when nothing is pending.
const SPURIOUS_INTID: u32 = 1023;

/// The system interrupt controller.
pub struct IrqChip {
    gicd: usize,
    gicc: usize,
}

impl IrqChip {
    fn read(&self, base: usize, offset: usize) -> u32 {
        // SAFETY: GIC registers are mapped through the linear mapping.
        unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
    }

    fn write(&self, base: usize, offset: usize, value: u32) {
        // SAFETY: GIC registers are mapped through the linear mapping.
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) };
    }

    /// Initializes the distributor and this CPU's interface.
    fn init(&self) {
        // Enable the distributor.
        self.write(self.gicd, GICD_CTLR, 1);
        // Allow interrupts of all priorities and enable the CPU interface.
        self.write(self.gicc, GICC_PMR, 0xff);
        self.write(self.gicc, GICC_CTLR, 1);
    }

    /// Enables the given interrupt ID and routes it to the current CPU.
    pub(in crate::arch) fn enable(&self, intid: u8) {
        let intid = intid as usize;
        // Priority 0xa0 (mid-range), so it is not masked by PMR = 0xff.
        // SAFETY: byte-addressed priority register.
        unsafe {
            core::ptr::write_volatile((self.gicd + GICD_IPRIORITYR + intid) as *mut u8, 0xa0);
        }
        // Route shared peripheral interrupts (>= 32) to CPU 0. (PPIs/SGIs are
        // banked per-CPU and ignore the target register.)
        if intid >= 32 {
            // SAFETY: byte-addressed target register.
            unsafe {
                core::ptr::write_volatile((self.gicd + GICD_ITARGETSR + intid) as *mut u8, 0x01);
            }
        }
        let reg = (intid / 32) * 4;
        let bit = 1u32 << (intid % 32);
        self.write(self.gicd, GICD_ISENABLER + reg, bit);
    }

    /// Disables the given interrupt ID.
    pub(in crate::arch) fn disable(&self, intid: u8) {
        let intid = intid as usize;
        let reg = (intid / 32) * 4;
        let bit = 1u32 << (intid % 32);
        self.write(self.gicd, GICD_ICENABLER + reg, bit);
    }

    /// Sends a software-generated interrupt (SGI) `intid` to the CPU whose GIC
    /// CPU-interface number is `target_cpu`.
    pub(in crate::arch) fn send_sgi(&self, intid: u8, target_cpu: u8) {
        let val = ((1u32 << (target_cpu as u32)) << 16) | (intid as u32 & 0xf);
        self.write(self.gicd, GICD_SGIR, val);
    }

    /// Claims the highest-priority pending interrupt, returning its ID.
    pub(in crate::arch) fn claim_interrupt(&self) -> Option<u8> {
        let iar = self.read(self.gicc, GICC_IAR) & 0x3ff;
        if iar == SPURIOUS_INTID {
            None
        } else {
            Some(iar as u8)
        }
    }

    /// Signals completion of a previously claimed interrupt.
    pub(in crate::arch) fn complete_interrupt(&self, intid: u8) {
        self.write(self.gicc, GICC_EOIR, intid as u32);
    }
}

/// A device-tree-described interrupt source.
pub struct InterruptSourceInFdt {
    _private: (),
}

/// An IRQ line mapped from a device-tree interrupt specifier.
pub struct MappedIrqLine {
    _private: (),
}

/// The global interrupt-controller instance.
pub static IRQ_CHIP: Once<IrqChip> = Once::new();

/// Initializes the interrupt controller on the BSP.
///
/// # Safety
///
/// Must be called once on the BSP during boot, before any interrupt-related
/// operation.
pub(in crate::arch) unsafe fn init_on_bsp() {
    let chip = IrqChip {
        gicd: paddr_to_vaddr(GICD_BASE_PADDR),
        gicc: paddr_to_vaddr(GICC_BASE_PADDR),
    };
    chip.init();
    IRQ_CHIP.call_once(|| chip);
}

/// Initializes the interrupt controller on an AP.
///
/// # Safety
///
/// Must be called once on each AP during boot.
pub(in crate::arch) unsafe fn init_on_ap() {
    if let Some(chip) = IRQ_CHIP.get() {
        // Each CPU must enable its own CPU interface.
        chip.write(chip.gicc, GICC_PMR, 0xff);
        chip.write(chip.gicc, GICC_CTLR, 1);
    }
}
