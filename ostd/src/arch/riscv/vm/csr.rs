// riscv crate does not support hypervisor csrs(hgatp...)
// so we impl this by ourself

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
                unsafe { asm!(concat!("csrr {0}, ", stringify!($addr)), out(reg) r) };
                r
            }

            /// Write value to csr
            #[inline]
            pub(super) unsafe fn write(value: usize) {
                unsafe { asm!(concat!("csrw ", stringify!($addr), ", {0}"), in(reg) value) };
            }
            
            /// Set bits to csr
            #[inline]
            pub(super) unsafe fn set_bits(mask: usize) {
                unsafe {
                    let current = Self::read();
                    Self::write(current | mask);
                }

            }
            
            /// Clear csr
            #[inline]
            pub(super) unsafe fn clear_bits(mask: usize) {
                unsafe {
                    let current = Self::read();
                    Self::write(current & !mask);
                }
            }

            /// Swap value with csr
            #[inline]
            pub(super) unsafe fn swap(value: usize) -> usize {
                let old: usize;
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

pub(super) const SR_SPIE:usize = 0x00000020;
pub(super) const SR_MPIE:usize = 0x00000080;
pub(super) const SR_SPP:usize = 0x00000100;

pub(super) const HSTATUS_VTW:usize = 0x00200000;
pub(super) const HSTATUS_SPVP:usize = 0x00000100;
/// 1
pub const HSTATUS_SPV:usize = 0x00000080;


pub(super) const ENVCFG_STCE:usize = 0x1 << 63;
pub(super) const ENVCFG_ADUE:usize = 0x1 << 61;
pub(super) const ENVCFG_CBIE:usize = 0x3 << 4;
pub(super) const ENVCFG_CBCFE:usize = 0x1 << 6;
pub(super) const ENVCFG_CBZE:usize = 0x1 << 7;
