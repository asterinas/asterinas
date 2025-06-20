// SPDX-License-Identifier: MPL-2.0

//! Memory Protection Keys (MPK) support.
//!
//! Intel memory protection keys are also known as Protection Keys for User-
//! mode pages (PKU) in the Intel architecture. AMD processors provide the same
//! functionality.
//!
//! The ARM architecture provides a similar feature called Memory Tagging
//! Extension (MTE), which should be supported here in the future.
//!
//! This module provides architecture-agnostic support for protection keys.

bitflags::bitflags! {
    pub struct PKeyAccessRights: u32 {
        const PKEY_DISABLE_ACCESS   = 0x1;
        const PKEY_DISABLE_WRITE    = 0x2;
    }
}

/// The protection key for user pages.
pub type PKey = u32;

pub fn is_pkey_supported() -> bool {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            // Check if the processor supports MPK.
            let cpuid = ostd::cpu::context::cpuid::CpuId::new();
            let feature_info = cpuid.get_extended_feature_info().unwrap();
            feature_info.has_pku()
        } else {
            // Do not support MPK in other architectures currently.
            false
        }
    }
}

const NR_PKEYS: PKey = 16;
type PKeyBitMap = u16;
const PKEY_BITMAP_MASK: PKeyBitMap = ((1_usize << NR_PKEYS as usize) - 1) as PKeyBitMap;

/// An allocator for protection keys associated with the [`super::Vmar`].
#[derive(Debug, Clone)]
pub(super) struct PKeyAllocator {
    /// "1" bits are allocated protection keys. Bit 0 corresponds to
    /// protection key 0, which is the default key for all memory pages.
    /// This 0 bit must be set to 1.
    bitmap: PKeyBitMap,
    /// 0: unknown, 1: not supported, 2: supported
    is_supported: u8,
}

pub(super) enum PKeyAllocError {
    /// The maximum number of protection keys has been reached.
    MaxPKeyReached,
    /// The processor does not support MPK.
    NotSupported,
}

pub(super) enum PKeyFreeError {
    /// The protection key is not allocated.
    NotAllocated,
    /// The provided protection key is out of range or is 0.
    Invalid,
    /// The processor does not support MPK.
    NotSupported,
}

impl PKeyAllocator {
    pub(super) const fn new() -> Self {
        Self {
            bitmap: 0b1,
            is_supported: 0,
        }
    }

    /// Allocates a new protection key for the given access rights.
    ///
    /// The provided access rights will be written to the current task's
    /// protection key register. Calling this function from another task
    /// leads to undefined behavior.
    pub(super) fn alloc(
        &mut self,
        rights: PKeyAccessRights,
    ) -> core::result::Result<PKey, PKeyAllocError> {
        if !self.is_supported() {
            return Err(PKeyAllocError::NotSupported);
        }
        if self.bitmap == PKEY_BITMAP_MASK {
            return Err(PKeyAllocError::MaxPKeyReached);
        }

        let pkey = self.bitmap.trailing_ones() as PKey;
        self.bitmap |= 1 << pkey;

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                // Set the PKRU register to set the access rights for the new
                // protection key.
                let old_pkru = ostd::cpu::read_pkru();
                let new_pkru = (old_pkru & !(0b11 << (pkey * 2))) | (rights.bits() << (pkey * 2));

                // `wrpkru` is much slower than `rdpkru`, so we only write it if the
                // PKRU value has changed.
                if old_pkru != new_pkru {
                    ostd::cpu::write_pkru(new_pkru);
                }
            } else {
                // Do nothing for other architectures.
            }
        }

        Ok(pkey)
    }

    /// Frees a protection key.
    pub(super) fn free(&mut self, pkey: PKey) -> core::result::Result<(), PKeyFreeError> {
        if !self.is_supported() {
            return Err(PKeyFreeError::NotSupported);
        }
        if pkey == 0 || pkey >= NR_PKEYS {
            return Err(PKeyFreeError::Invalid);
        }
        if self.bitmap & (1 << pkey) == 0 {
            return Err(PKeyFreeError::NotAllocated);
        }

        self.bitmap &= !(1 << pkey);

        Ok(())
    }

    /// Checks if a protection key is allocated.
    pub(super) fn is_allocated(&mut self, pkey: PKey) -> bool {
        if !self.is_supported() {
            return false;
        }
        if pkey == 0 || pkey >= NR_PKEYS {
            return false;
        }
        self.bitmap & (1 << pkey) != 0
    }

    fn is_supported(&mut self) -> bool {
        if self.is_supported == 0 {
            self.is_supported = if is_pkey_supported() { 2 } else { 1 };
        }
        self.is_supported == 2
    }
}
