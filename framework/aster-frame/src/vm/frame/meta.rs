// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::Deref,
    sync::atomic::{AtomicU16, AtomicU32, AtomicU8},
};

use static_assertions::const_assert_eq;

use crate::{
    arch::mm::PagingConsts,
    vm::{page_size, Paddr, PagingLevel},
};

pub mod mapping {
    //! The metadata of each physical frame is mapped to fixed virtual addresses
    //! from [`FRAME_METADATA_BASE_VADDR`] to [`FRAME_METADATA_CAP_VADDR`]. This
    //! range is divided into portions in the geometric series of 1/2, which are
    //! 1/2, 1/4, 1/8, ..., 1/2^N. The Nth portion is used to store the metadata
    //! of the level N pages.

    use core::mem::size_of;

    use super::FrameMeta;
    use crate::vm::{
        kspace::{FRAME_METADATA_BASE_VADDR, FRAME_METADATA_CAP_VADDR},
        page_size, Paddr, PagingConstsTrait, PagingLevel, Vaddr,
    };

    /// Convert a physical address of the page to the virtual address of the metadata container.
    pub const fn page_to_meta<C: PagingConstsTrait>(paddr: Paddr, level: PagingLevel) -> Vaddr {
        let kvspace = FRAME_METADATA_CAP_VADDR - FRAME_METADATA_BASE_VADDR;
        let base = FRAME_METADATA_CAP_VADDR - (kvspace >> (level - 1));
        let offset = paddr / page_size::<C>(level);
        base + offset * size_of::<FrameMeta>()
    }

    /// Convert a virtual address of the metadata container to the physical address of the page.
    pub const fn meta_to_page<C: PagingConstsTrait>(vaddr: Vaddr) -> Paddr {
        let kvspace = FRAME_METADATA_CAP_VADDR - FRAME_METADATA_BASE_VADDR;
        let level = level_of_meta(vaddr);
        let base = FRAME_METADATA_CAP_VADDR - (kvspace >> (level - 1));
        let offset = (vaddr - base) / size_of::<FrameMeta>();
        offset * page_size::<C>(level)
    }

    /// Get the level of the page from the address of the metadata container.
    pub const fn level_of_meta(vaddr: Vaddr) -> PagingLevel {
        let kvspace = FRAME_METADATA_CAP_VADDR - FRAME_METADATA_BASE_VADDR;
        (kvspace.ilog2() - (FRAME_METADATA_CAP_VADDR - (vaddr + 1)).ilog2()) as PagingLevel
    }

    #[cfg(ktest)]
    #[ktest]
    fn test_meta_mapping() {
        use crate::arch::mm::PagingConsts;
        for level in 1..=3 {
            let meta = page_to_meta::<PagingConsts>(0, level);
            assert_eq!(meta_to_page::<PagingConsts>(meta), 0);
            assert_eq!(level_of_meta(meta), level);
            let paddr = 123456 * page_size::<PagingConsts>(level);
            let meta = page_to_meta::<PagingConsts>(paddr, level);
            assert_eq!(meta_to_page::<PagingConsts>(meta), paddr);
            assert_eq!(level_of_meta(meta), level);
        }
    }
}

/// A reference to the metadata of a physical frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FrameMetaRef {
    // FIXME: this shouldn't be public but XArray needs it.
    pub(crate) inner: *const FrameMeta,
}

impl FrameMetaRef {
    /// Create a new reference to the metadata of a raw frame.
    ///
    /// # Safety
    ///
    /// The caller must ensure that
    ///  - the metadata is initialized before any access;
    ///  - the super page, if would be used, must be splitted.
    pub unsafe fn from_raw(paddr: Paddr, level: PagingLevel) -> Self {
        debug_assert_eq!(paddr % page_size::<PagingConsts>(level), 0);
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr, level);
        Self {
            inner: vaddr as *const FrameMeta,
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that the reference is the exclusive.
    pub unsafe fn deref_mut(&mut self) -> &mut FrameMeta {
        &mut *(self.inner as *mut FrameMeta)
    }

    /// Get the physical address of the frame.
    pub fn paddr(&self) -> Paddr {
        mapping::meta_to_page::<PagingConsts>(self.inner as usize)
    }

    /// Get the level of the page.
    pub fn level(&self) -> PagingLevel {
        mapping::level_of_meta(self.inner as usize)
    }

    /// Get the size of the frame.
    pub fn size(&self) -> usize {
        page_size::<PagingConsts>(self.level())
    }
}

impl Deref for FrameMetaRef {
    type Target = FrameMeta;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The metadata container is ensured initialized before any access.
        unsafe { &*self.inner }
    }
}

/// The metadata of a physical frame.
///
/// If a physical frame exists, the unique metadata must be initialized somewhere
/// just for it. The place decided by the schema defined in [`mapping`];
///
/// The zero value of the metadata must be valid and it's used as the initial state
/// of a frame.
#[repr(C)]
pub struct FrameMeta {
    pub frame_type: FrameType, // 1 byte
    /// The first 8-bit counter.
    ///  - For [`FrameType::Anonymous`], it is not used.
    ///  - For [`FrameType::PageTable`], it is used as a spinlock.
    pub counter8_1: AtomicU8, // 1 byte
    /// The first 16-bit counter.
    ///  - For [`FrameType::Anonymous`], it is not used.
    ///  - For [`FrameType::PageTable`], it is used as the map count. The map
    ///    count is the number of present children.
    pub counter16_1: AtomicU16, // 2 bytes
    /// The first 32-bit counter.
    ///  - For [`FrameType::Anonymous`], it is the handle count.
    ///  - For [`FrameType::PageTable`], it is used as the reference count. The referencer
    ///    can be either a handle, a PTE or a CPU that loads it.
    pub counter32_1: AtomicU32, // 4 bytes
}

const_assert_eq!(core::mem::size_of::<FrameMeta>(), 8);

#[repr(u8)]
pub enum FrameType {
    Free = 0,
    /// The frame allocated to store metadata.
    Meta,
    Anonymous,
    PageTable,
    /// Frames that contains kernel code.
    KernelCode,
}
