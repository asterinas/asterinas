// SPDX-License-Identifier: MPL-2.0

//! FPU context and state management.

use alloc::boxed::Box;
use core::arch::x86_64::{_fxrstor64, _fxsave64, _xrstor64, _xsave64};

use ostd_pod::{FromZeros, IntoBytes};
use spin::Once;
use x86_64::registers::{
    control::{Cr0, Cr0Flags},
    xcontrol::XCr0,
};

use crate::debug;

/// The FPU context of user task.
///
/// This could be used for saving both legacy and modern state format.
#[derive(Debug)]
pub struct FpuContext {
    xsave_area: Box<XSaveArea>,
    area_size: usize,
}

impl FpuContext {
    /// Creates a new FPU context.
    pub fn new() -> Self {
        let mut area_size = size_of::<FxSaveArea>();
        if let Some(xsave_area_size) = XSAVE_AREA_SIZE.get() {
            area_size = area_size.max(*xsave_area_size);
        }

        Self {
            xsave_area: Box::new(XSaveArea::new()),
            area_size,
        }
    }

    /// Saves CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        let mem_addr = self.as_bytes_mut().as_mut_ptr();

        if XSTATE_MAX_FEATURES.is_completed() {
            unsafe { _xsave64(mem_addr, XFEATURE_MASK_USER_RESTORE) };
        } else {
            unsafe { _fxsave64(mem_addr) };
        }

        debug!("Save FPU context");
    }

    /// Loads CPU's FPU context from this instance.
    pub fn load(&self) {
        let mem_addr = self.as_bytes().as_ptr();

        if let Some(xstate_max_features) = XSTATE_MAX_FEATURES.get() {
            let rs_mask = XFEATURE_MASK_USER_RESTORE & *xstate_max_features;

            unsafe { _xrstor64(mem_addr, rs_mask) };
        } else {
            unsafe { _fxrstor64(mem_addr) };
        }

        debug!("Load FPU context");
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.xsave_area.as_bytes()[..self.area_size]
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.xsave_area.as_mut_bytes()[..self.area_size]
    }
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for FpuContext {
    fn clone(&self) -> Self {
        let mut xsave_area = Box::new(XSaveArea::new());
        xsave_area.fxsave_area = self.xsave_area.fxsave_area;
        xsave_area.features = self.xsave_area.features;
        xsave_area.compaction = self.xsave_area.compaction;
        if self.area_size > size_of::<FxSaveArea>() {
            let len = self.area_size - size_of::<FxSaveArea>() - 64;
            xsave_area.extended_state_area[..len]
                .copy_from_slice(&self.xsave_area.extended_state_area[..len]);
        }

        Self {
            xsave_area,
            area_size: self.area_size,
        }
    }
}

/// The modern FPU context format (as saved and restored by the `XSAVE` and `XRSTOR` instructions).
#[repr(C)]
#[repr(align(64))]
#[derive(Clone, Copy, Debug, Pod)]
struct XSaveArea {
    fxsave_area: FxSaveArea,
    features: u64,
    compaction: u64,
    reserved: [u64; 6],
    extended_state_area: [u8; MAX_XSAVE_AREA_SIZE - size_of::<FxSaveArea>() - 64],
}

impl XSaveArea {
    fn new() -> Self {
        let features = if let Some(xstate_max_features) = XSTATE_MAX_FEATURES.get() {
            XCr0::read().bits() & *xstate_max_features
        } else {
            0
        };

        let mut xsave_area = Self::new_zeroed();
        // Set the initial values for the FPU context. Refer to Intel SDM, Table 10-1:
        // "IA-32 and Intel® 64 Processor States Following Power-up, Reset, or INIT (Contd.)".
        xsave_area.fxsave_area.control = 0x037F;
        // Refer to Intel SDM, Volume 1, Section "x87 State". In the FXSAVE/XSAVE image the
        // `tag` field contains an abridged x87 Tag Word (FTW) - a compact (8-bit) encoding
        // used in saved/restore images. In this format, a bit value of 0 indicates the
        // corresponding x87 register is empty (this is the inverse of the legacy 16-bit
        // tag-word semantics). The `fninit` instruction clears all x87 registers, so the
        // abridged tag must be initialized to 0 to represent an empty register set.
        xsave_area.fxsave_area.tag = 0;
        xsave_area.fxsave_area.mxcsr = 0x1F80;
        xsave_area.features = features;

        xsave_area
    }
}

/// The legacy SSE/MMX FPU context format (as saved and restored by the `FXSAVE` and `FXRSTOR` instructions).
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Pod)]
struct FxSaveArea {
    control: u16,         // x87 FPU Control Word
    status: u16,          // x87 FPU Status Word
    tag: u8,              // x87 FPU Tag Byte (abridged format)
    reserved1: u8,        // Reserved
    op: u16,              // x87 FPU Last Instruction Opcode
    ip: u32,              // x87 FPU Instruction Pointer Offset
    cs: u32,              // x87 FPU Instruction Pointer Selector
    dp: u32,              // x87 FPU Instruction Operand (Data) Pointer Offset
    ds: u32,              // x87 FPU Instruction Operand (Data) Pointer Selector
    mxcsr: u32,           // MXCSR Register State
    mxcsr_mask: u32,      // MXCSR Mask
    st_space: [u32; 32], // x87 FPU or MMX technology registers (ST0-ST7 or MM0-MM7, 128 bits per field)
    xmm_space: [u32; 64], // XMM registers (XMM0-XMM15, 128 bits per field)
    reserved2: [u32; 12], // Reserved
    reserved3: [u32; 12], // Software reserved
}

/// The XSTATE features (user & supervisor) supported by the processor.
static XSTATE_MAX_FEATURES: Once<u64> = Once::new();

/// Mask features which are restored when returning to user space.
///
/// X87 | SSE | AVX | OPMASK | ZMM_HI256 | HI16_ZMM
const XFEATURE_MASK_USER_RESTORE: u64 = 0b1110_0111;

/// The real size in bytes of the XSAVE area containing all states enabled by XCRO | IA32_XSS.
static XSAVE_AREA_SIZE: Once<usize> = Once::new();

/// The max size in bytes of the XSAVE area.
const MAX_XSAVE_AREA_SIZE: usize = 4096;

pub(in crate::arch) fn enable_essential_features() {
    use super::super::extension::{IsaExtensions, has_extensions};

    if has_extensions(IsaExtensions::XSAVE) {
        XSTATE_MAX_FEATURES.call_once(|| super::super::cpuid::query_xstate_max_features().unwrap());
        XSAVE_AREA_SIZE.call_once(|| {
            let xsave_area_size = super::super::cpuid::query_xsave_area_size().unwrap() as usize;
            assert!(xsave_area_size <= MAX_XSAVE_AREA_SIZE);
            xsave_area_size
        });
    }

    // We now assume that all x86-64 CPUs should have the FPU. Otherwise, we should check
    // `has_extensions(IsaExtensions::FPU)` here.
    {
        let mut cr0 = Cr0::read();
        cr0.remove(Cr0Flags::TASK_SWITCHED | Cr0Flags::EMULATE_COPROCESSOR);

        unsafe {
            Cr0::write(cr0);
            // Flush out any pending x87 state.
            core::arch::asm!("fninit");
        }
    }
}
