// SPDX-License-Identifier: MPL-2.0

//! Validation and import of FXSAVE/XSAVE register images.

use ostd_pod::IntoBytes;

use super::{FpuContext, FxSaveArea};
use crate::{Error, Result};

const X87: u64 = 1 << 0;
const SSE: u64 = 1 << 1;
const AVX: u64 = 1 << 2;
const FXSAVE_SIZE: usize = size_of::<FxSaveArea>();
const MXCSR_OFFSET: usize = core::mem::offset_of!(FxSaveArea, mxcsr);
const X87_REGS_OFFSET: usize = core::mem::offset_of!(FxSaveArea, st_space);
const XMM_OFFSET: usize = core::mem::offset_of!(FxSaveArea, xmm_space);
const XMM_END: usize = core::mem::offset_of!(FxSaveArea, reserved2);
const XCOMP_BV_COMPACTED: u64 = 1 << 63;
const XSAVE_ALIGNMENT: usize = align_of::<super::XSaveArea>();

impl FpuContext {
    /// Reads and validates a register image for the current FPU configuration.
    ///
    /// The input uses FXSAVE format when XSAVE is disabled,
    /// and XSAVE format otherwise.
    /// `read_fn` reads bytes at offsets relative to the image.
    /// The returned context uses the same format as [`Self::save`].
    /// Invalid state returns [`Error::InvalidArgs`]; read errors are propagated.
    pub fn read_from<F>(mut read_fn: F) -> Result<Self>
    where
        F: FnMut(usize, &mut [u8]) -> Result<()>,
    {
        let config = super::fpu_config();
        let mut context = Self::new();
        if !config.has_xsave {
            read_fn(0, context.xsave_area.fxsave_area.as_mut_bytes())?;
            if context.xsave_area.fxsave_area.mxcsr & !config.mxcsr_mask != 0 {
                return Err(Error::InvalidArgs);
            }
            // MXCSR_MASK describes this CPU, not state controlled by the caller.
            context.xsave_area.fxsave_area.mxcsr_mask = config.mxcsr_mask;
            return Ok(context);
        }

        let mut header = [0u8; super::XSAVE_HEADER_SIZE];
        read_fn(FXSAVE_SIZE, &mut header)?;
        let xstate_bv = u64::from_le_bytes(header[..8].try_into().unwrap());
        let xcomp_bv = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let is_compacted = xcomp_bv & XCOMP_BV_COMPACTED != 0;
        // Intel SDM Volume 1, "Operation of XRSTOR": validate the source header
        // before converting it to the standard format used by this context.
        if is_compacted {
            if !config.has_compacted_xrstor
                || xcomp_bv & !(config.user_features | XCOMP_BV_COMPACTED) != 0
                || xstate_bv & !xcomp_bv != 0
                || xstate_bv & XCOMP_BV_COMPACTED != 0
                || header[16..].iter().any(|&byte| byte != 0)
            {
                return Err(Error::InvalidArgs);
            }
        } else if xstate_bv & !config.user_features != 0
            || header[8..24].iter().any(|&byte| byte != 0)
        {
            return Err(Error::InvalidArgs);
        }

        let bytes = context.xsave_area.as_mut_bytes();
        if xstate_bv & X87 != 0 {
            read_fn(0, &mut bytes[..MXCSR_OFFSET])?;
            read_fn(X87_REGS_OFFSET, &mut bytes[X87_REGS_OFFSET..XMM_OFFSET])?;
        }
        if xstate_bv & SSE != 0 {
            read_fn(XMM_OFFSET, &mut bytes[XMM_OFFSET..XMM_END])?;
        }
        // Standard XRSTOR can load MXCSR even when SSE is in its initial state.
        let loads_mxcsr = if is_compacted {
            xstate_bv & SSE != 0
        } else {
            config.user_features & (SSE | AVX) != 0
        };
        if loads_mxcsr {
            let mut mxcsr_bytes = [0u8; size_of::<u32>()];
            read_fn(MXCSR_OFFSET, &mut mxcsr_bytes)?;
            if u32::from_le_bytes(mxcsr_bytes) & !config.mxcsr_mask != 0 {
                return Err(Error::InvalidArgs);
            }
            bytes[MXCSR_OFFSET..MXCSR_OFFSET + mxcsr_bytes.len()].copy_from_slice(&mxcsr_bytes);
        }

        let mut compacted_offset = super::XSAVE_HEADER_END;
        // Bit 63 selects the compacted format; it is not a state component.
        for index in 2..63 {
            let component = config.components[index];
            let input_offset = if is_compacted && xcomp_bv & (1 << index) != 0 {
                if component.needs_64_byte_alignment {
                    compacted_offset =
                        (compacted_offset + XSAVE_ALIGNMENT - 1) & !(XSAVE_ALIGNMENT - 1);
                }
                let offset = compacted_offset;
                compacted_offset = compacted_offset
                    .checked_add(component.size)
                    .ok_or(Error::Overflow)?;
                offset
            } else {
                component.offset
            };
            if xstate_bv & (1 << index) != 0 {
                let end = component.offset + component.size;
                read_fn(input_offset, &mut bytes[component.offset..end])?;
            }
        }
        context.xsave_area.xstate_bv = xstate_bv;
        Ok(context)
    }
}
