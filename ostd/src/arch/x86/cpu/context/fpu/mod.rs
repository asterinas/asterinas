// SPDX-License-Identifier: MPL-2.0

//! FPU context and state management.

mod import;

use alloc::boxed::Box;
use core::arch::x86_64::{_fxrstor64, _fxsave64, _xrstor64, _xsave64};

use ostd_pod::{FromZeros, IntoBytes};
use spin::Once;
use x86_64::registers::{
    control::{Cr0, Cr0Flags},
    xcontrol::XCr0,
};

use super::super::cpuid;
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
        let area_size = fpu_config().area_size;

        Self {
            xsave_area: Box::new(XSaveArea::new()),
            area_size,
        }
    }

    /// Saves the current CPU's FPU registers.
    ///
    /// The caller is responsible for ensuring that the registers contain the
    /// state to be saved.
    pub fn save(&mut self) {
        let mem_addr = self.xsave_area.as_mut_bytes().as_mut_ptr();

        if fpu_config().has_xsave {
            // SAFETY: The configured user state fits in this aligned area.
            unsafe { _xsave64(mem_addr, fpu_config().user_features) };
        } else {
            // SAFETY: The area is aligned and large enough for an FXSAVE image.
            unsafe { _fxsave64(mem_addr) };
        }

        debug!("Save FPU context");
    }

    /// Loads this context into the current CPU's FPU registers.
    ///
    /// The caller is responsible for managing FPU register ownership.
    pub fn load(&self) {
        let mem_addr = self.as_bytes().as_ptr();

        if fpu_config().has_xsave {
            // SAFETY: The area is aligned and contains a validated XSAVE image.
            unsafe { _xrstor64(mem_addr, fpu_config().user_features) };
        } else {
            // SAFETY: The area is aligned and contains a validated FXSAVE image.
            unsafe { _fxrstor64(mem_addr) };
        }

        debug!("Load FPU context");
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.xsave_area.as_bytes()[..self.area_size]
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
        xsave_area.xstate_bv = self.xsave_area.xstate_bv;
        xsave_area.xcomp_bv = self.xsave_area.xcomp_bv;
        if self.area_size > size_of::<FxSaveArea>() {
            let len = self.area_size - XSAVE_HEADER_END;
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
    // Components in a non-initial state.
    xstate_bv: u64,
    // Compacted-format flag and component-layout bitmap; zero internally.
    xcomp_bv: u64,
    reserved: [u64; 6],
    extended_state_area: [u8; MAX_XSAVE_AREA_SIZE - XSAVE_HEADER_END],
}

impl XSaveArea {
    fn new() -> Self {
        let mut xsave_area = Self::new_zeroed();
        xsave_area.fxsave_area = FxSaveArea::initial();
        xsave_area
    }
}

impl FxSaveArea {
    fn initial() -> Self {
        let mut fxsave_area = Self::new_zeroed();
        // Set the initial values for the FPU context. Refer to Intel SDM, Table 10-1:
        // "IA-32 and Intel® 64 Processor States Following Power-up, Reset, or INIT (Contd.)".
        fxsave_area.control = 0x037F;
        // Refer to Intel SDM, Volume 1, Section "x87 State". In the FXSAVE/XSAVE image the
        // `tag` field contains an abridged x87 Tag Word (FTW) - a compact (8-bit) encoding
        // used in saved/restore images. In this format, a bit value of 0 indicates the
        // corresponding x87 register is empty (this is the inverse of the legacy 16-bit
        // tag-word semantics). The `fninit` instruction clears all x87 registers, so the
        // abridged tag must be initialized to 0 to represent an empty register set.
        fxsave_area.tag = 0;
        fxsave_area.mxcsr = INITIAL_MXCSR;
        fxsave_area.mxcsr_mask = fpu_config().mxcsr_mask;
        fxsave_area
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
    rip: u64,             // x87 FPU Instruction Pointer (FXSAVE64 format)
    rdp: u64,             // x87 FPU Data Pointer (FXSAVE64 format)
    mxcsr: u32,           // MXCSR Register State
    mxcsr_mask: u32,      // MXCSR Mask
    st_space: [u32; 32], // x87 FPU or MMX technology registers (ST0-ST7 or MM0-MM7, 128 bits per field)
    xmm_space: [u32; 64], // XMM registers (XMM0-XMM15, 128 bits per field)
    reserved2: [u32; 12], // Reserved
    reserved3: [u32; 12], // Software reserved
}

/// Mask features which are restored when returning to user space.
///
/// X87 | SSE | AVX | OPMASK | ZMM_HI256 | HI16_ZMM
const XFEATURE_MASK_USER_RESTORE: u64 = 0b1110_0111;

const XSAVE_HEADER_SIZE: usize = 64;
const XSAVE_HEADER_END: usize = size_of::<FxSaveArea>() + XSAVE_HEADER_SIZE;

// Intel SDM Volume 1, "Guidelines for Writing to the MXCSR Register".
const DEFAULT_MXCSR_MASK: u32 = 0x0000_ffbf;
const INITIAL_MXCSR: u32 = 0x1f80;

/// The max size in bytes of the XSAVE area.
const MAX_XSAVE_AREA_SIZE: usize = 4096;

// CPUID leaf 0xD: subleaf 1 EAX and component subleaf ECX flags.
const XSAVE_CAP_XSAVEC: u32 = 1 << 1;
const XSTATE_COMPONENT_ALIGNED64: u32 = 1 << 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct XstateComponent {
    offset: usize,
    size: usize,
    needs_64_byte_alignment: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FpuConfig {
    has_xsave: bool,
    has_compacted_xrstor: bool,
    user_features: u64,
    // Effective mask after applying the architectural fallback for a zero mask.
    mxcsr_mask: u32,
    area_size: usize,
    components: [XstateComponent; 64],
}

static FPU_CONFIG: Once<FpuConfig> = Once::new();

fn fpu_config() -> &'static FpuConfig {
    FPU_CONFIG.get().expect("FPU has not been initialized")
}

pub(in crate::arch) fn enable_essential_features() {
    use super::super::extension::{self, IsaExtensions};

    // We now assume that all x86-64 CPUs should have the FPU. Otherwise, we should check
    // `extension::has_extensions(IsaExtensions::FPU)` here.
    {
        let mut cr0 = Cr0::read();
        cr0.remove(Cr0Flags::TASK_SWITCHED | Cr0Flags::EMULATE_COPROCESSOR);

        // SAFETY: This runs during CPU initialization, before this CPU runs
        // user code. Clearing TS and EM enables the FPU without changing the other
        // CR0 flags; FNINIT initializes the newly enabled x87 state.
        unsafe {
            Cr0::write(cr0);
            // Flush out any pending x87 state.
            core::arch::asm!("fninit");
        }
    }

    let mut fxsave_area = FxSaveArea::new_zeroed();
    // SAFETY: The FPU has been enabled above and `FxSaveArea` is 16-byte aligned.
    unsafe { _fxsave64(fxsave_area.as_mut_bytes().as_mut_ptr()) };
    let mxcsr_mask = if fxsave_area.mxcsr_mask == 0 {
        DEFAULT_MXCSR_MASK
    } else {
        fxsave_area.mxcsr_mask
    };
    let has_xsave = extension::has_extensions(IsaExtensions::XSAVE);
    let user_features = if has_xsave {
        XCr0::read().bits() & XFEATURE_MASK_USER_RESTORE
    } else {
        0b11
    };
    let mut area_size = size_of::<FxSaveArea>();
    let mut components = [XstateComponent::default(); 64];
    if has_xsave {
        // CPUID reports the size for every XCR0-enabled component, including
        // states that this kernel does not save. Size our image only for the
        // components in the restore mask, and verify each against both bounds.
        let xcr0_area_size = cpuid::query_xsave_area_size().unwrap() as usize;
        assert!(xcr0_area_size >= XSAVE_HEADER_END);
        area_size = XSAVE_HEADER_END;
        for (index, slot) in components.iter_mut().enumerate().skip(2) {
            if user_features & (1 << index) == 0 {
                continue;
            }
            let info = cpuid::query_xstate_component(index as u32).unwrap();
            let component = XstateComponent {
                offset: info.ebx as usize,
                size: info.eax as usize,
                needs_64_byte_alignment: info.ecx & XSTATE_COMPONENT_ALIGNED64 != 0,
            };
            assert!(component.size > 0 && component.offset >= XSAVE_HEADER_END);
            let end = component.offset.checked_add(component.size).unwrap();
            assert!(end <= xcr0_area_size && end <= MAX_XSAVE_AREA_SIZE);
            area_size = area_size.max(end);
            *slot = component;
        }
    }
    let xsave_caps = if has_xsave {
        cpuid::query_xsave_capabilities().unwrap()
    } else {
        0
    };
    let config = FpuConfig {
        has_xsave,
        has_compacted_xrstor: xsave_caps & XSAVE_CAP_XSAVEC != 0,
        user_features,
        mxcsr_mask,
        area_size,
        components,
    };
    let expected = FPU_CONFIG.call_once(|| config);
    // Every CPU must be able to restore a context saved on another CPU.
    assert_eq!(*expected, config, "incompatible FPU configuration");
}
