// SPDX-License-Identifier: MPL-2.0

//! RISC-V H-extension CSRs.
//!
//! Control and Status Registers (CSRs) are the registers that control and report the state of a RISC-V hart.
//!
//! The hypervisor extension (H-extension) adds its own set of CSRs, such as `hstatus` and `hgatp`,
//! which are accessed with the same CSR instructions as any other CSR.
//!
//! The `riscv` crate does not yet implement read and write operations for these registers,
//! so this module implements them itself.
//!
//! TODO: If the `riscv` crate adds support for them in the future, this module can be removed.

use core::arch::asm;

macro_rules! define_csr {
    ($name:ident, $addr:expr) => {
        /// CSR register($name)
        #[derive(Clone, Copy)]
        pub(super) struct $name;

        impl $name {
            /// Read this csr
            #[inline]
            pub(super) unsafe fn read() -> usize {
                let r: usize;
                // SAFETY: Reading a CSR into a GPR has no memory side effects beyond the CSR itself;
                // `$addr` is a valid CSR address.
                unsafe { asm!(concat!("csrr {0}, ", stringify!($addr)), out(reg) r) };
                r
            }

            /// Write `value` to this CSR.
            #[inline]
            pub(super) unsafe fn write(value: usize) {
                // SAFETY: Writing a GPR to a CSR has no memory side effects beyond the CSR itself;
                // `$addr` is a valid CSR address.
                unsafe { asm!(concat!("csrw ", stringify!($addr), ", {0}"), in(reg) value) };
            }

            /// Atomically swap `value` into this CSR, returning the old value.
            #[inline]
            pub(super) unsafe fn swap(value: usize) -> usize {
                let old: usize;
                // SAFETY: `csrrw` atomically reads and writes the CSR.
                unsafe { asm!(concat!("csrrw {0}, ", stringify!($addr), ", {1}"), out(reg) old, in(reg) value) };
                old
            }
        }
    };
}

define_csr!(Vsstatus, 0x200);
define_csr!(Vsie, 0x204);
define_csr!(Vstvec, 0x205);
define_csr!(Vsscratch, 0x240);
define_csr!(Vsepc, 0x241);
define_csr!(Vscause, 0x242);
define_csr!(Vstval, 0x243);
define_csr!(Vsip, 0x244);
define_csr!(Vsatp, 0x280);
define_csr!(Vstimecmp, 0x24D);
define_csr!(Vstimecmph, 0x25D);

define_csr!(Hstatus, 0x600);
define_csr!(Hedeleg, 0x602);
define_csr!(Hideleg, 0x603);
define_csr!(Hie, 0x604);
define_csr!(Htimedelta, 0x605);
define_csr!(Hcounteren, 0x606);
define_csr!(Hgeie, 0x607);
define_csr!(Henvcfg, 0x60a);
define_csr!(Htimedeltah, 0x615);
define_csr!(Henvcfgh, 0x61a);
define_csr!(Htval, 0x643);
define_csr!(Hip, 0x644);
define_csr!(Hvip, 0x645);
define_csr!(Htinst, 0x64a);
define_csr!(Hgatp, 0x680);
define_csr!(Hgeip, 0xe12);

bitflags::bitflags! {
    /// ### WARNING!!!!!
    /// This register contains WPRI(Writes Preserve Values, Reads Ignore) bits.
    /// You must use `from_bits_unchecked` to instantiate this object;
    /// otherwise, the WPRI bits will be forced to zero.
    /// This would alter the register value upon write-back,
    /// which is highly likely to result in undefined behavior (UB).
    pub(super) struct HstatusFlags: usize {
        const VSBE = 0x1 << 5;
        const GVA = 0x1 << 6;
        const SPV = 0x1 << 7;
        const SPVP = 0x1 << 8;
        const HU = 0x1 << 9;
        const VGEIN = 0x3f << 12;
        const VTVM = 0x1 << 20;
        const VTW = 0x1 << 21;
        const VTSR = 0x1 << 22;
        const VSXL = 0x3 << 32;
        const HUPMM = 0x3 << 48;
    }
}

bitflags::bitflags! {
    /// ### WARNING!!!!!
    /// This register contains WPRI(Writes Preserve Values, Reads Ignore) bits.
    /// You must use `from_bits_unchecked` to instantiate this object;
    /// otherwise, the WPRI bits will be forced to zero.
    /// This would alter the register value upon write-back,
    /// which is highly likely to result in undefined behavior (UB).
    pub(super) struct HenvcfgFlags: usize {
        const FIOM   = 0x1 << 0;
        const LPE    = 0x1 << 2;
        const SSE    = 0x1 << 3;
        const CBIE = 0x3 << 4;
        const CBCFE = 0x1 << 6;
        const CBZE = 0x1 << 7;
        const PMM    = 0x3 << 32;
        const DTE    = 0x1 << 59;
        const ADUE = 0x1 << 61;
        const PBMTE  = 0x1 << 62;
        const STCE = 0x1 << 63;
    }
}
