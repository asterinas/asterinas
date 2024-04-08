// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Objects (VMOs).

use core::ops::Range;

use align_ext::AlignExt;
use aster_frame::{
    collections::xarray::{CursorMut, XArray, XMark},
    vm::{VmAllocOptions, VmFrame, VmFrameVec, VmIo},
};
use aster_rights::Rights;

use crate::prelude::*;

mod dyn_cap;
mod options;
mod pager;
mod static_cap;

pub use options::{VmoChildOptions, VmoOptions};
pub use pager::Pager;

use self::options::ChildType;

/// Virtual Memory Objects (VMOs) are a type of capability that represents a
/// range of memory pages.
///
/// # Features
///
/// * **I/O interface.** A VMO provides read and write methods to access the
/// memory pages that it contain.
/// * **On-demand paging.** The memory pages of a VMO (except for _contiguous_
/// VMOs) are allocated lazily when the page is first accessed.
/// * **Tree structure.** Given a VMO, one can create a child VMO from it.
/// The child VMO can only access a subset of the parent's memory,
/// which is a good thing for the perspective of access control.
/// * **Copy-on-write (COW).** A child VMO may be created with COW semantics,
/// which prevents any writes on the child from affecting the parent
/// by duplicating memory pages only upon the first writes.
/// * **Access control.** As capabilities, VMOs restrict the
/// accessible range of memory and the allowed I/O operations.
/// * **Device driver support.** If specified upon creation, VMOs will be
/// backed by physically contiguous memory pages starting at a target address.
/// * **File system support.** By default, a VMO's memory pages are initially
/// all zeros. But if a VMO is attached to a pager (`Pager`) upon creation,
/// then its memory pages will be populated by the pager.
/// With this pager mechanism, file systems can easily implement page caches
/// with VMOs by attaching the VMOs to pagers backed by inodes.
///
/// # Capabilities
///
/// As a capability, each VMO is associated with a set of access rights,
/// whose semantics are explained below.
///
/// * The Dup right allows duplicating a VMO and creating children out of
/// a VMO.
/// * The Read, Write, Exec rights allow creating memory mappings with
/// readable, writable, and executable access permissions, respectively.
/// * The Read and Write rights allow the VMO to be read from and written to
/// directly.
/// * The Write right allows resizing a resizable VMO.
///
/// VMOs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmo<Rights>`) and the static one (`Vmo<R: TRights>).
///
/// # Examples
///
/// For creating root VMOs, see `VmoOptions`.`
///
/// For creating child VMOs, see `VmoChildOptions`.
///
/// # Implementation
///
/// `Vmo` provides high-level APIs for address space management by wrapping
/// around its low-level counterpart `aster_frame::vm::VmFrames`.
/// Compared with `VmFrames`,
/// `Vmo` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
///
pub struct Vmo<R = Rights>(pub(super) Arc<Vmo_>, R);

/// Functions exist both for static capbility and dynamic capibility
pub trait VmoRightsOp {
    /// Returns the access rights.
    fn rights(&self) -> Rights;

    /// Check whether rights is included in self
    fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.rights().contains(rights) {
            Ok(())
        } else {
            return_errno_with_message!(Errno::EINVAL, "vmo rights check failed");
        }
    }

    /// Converts to a dynamic capability.
    fn to_dyn(self) -> Vmo<Rights>
    where
        Self: Sized;
}

// We implement this trait for VMO, so we can use functions on type like Vmo<R> without trait bounds.
// FIXME: This requires the imcomplete feature specialization, which should be fixed further.
impl<R> VmoRightsOp for Vmo<R> {
    default fn rights(&self) -> Rights {
        unimplemented!()
    }

    default fn to_dyn(self) -> Vmo<Rights>
    where
        Self: Sized,
    {
        unimplemented!()
    }
}

bitflags! {
    /// VMO flags.
    pub struct VmoFlags: u32 {
        /// Set this flag if a VMO is resizable.
        const RESIZABLE  = 1 << 0;
        /// Set this flags if a VMO is backed by physically contiguous memory
        /// pages.
        ///
        /// To ensure the memory pages to be contiguous, these pages
        /// are allocated upon the creation of the VMO, rather than on demands.
        const CONTIGUOUS = 1 << 1;
        /// Set this flag if a VMO is backed by memory pages that supports
        /// Direct Memory Access (DMA) by devices.
        const DMA        = 1 << 2;
    }
}

/// Marks used for the `XArray` in `Vmo_`.
#[derive(Copy, Clone)]
pub(super) enum VmoMark {
    /// Marks used for the VMO's `pages` which is managed by `XArray`.
    /// The VMO whose `pages` is marked as `CowVmo` may require a Copy-On-Write (COW) operation
    /// when performing a write action.
    CowVmo,
    /// Marks used for the `VmFrame` stored within the pages marked as `CowVmo`,
    /// `VmFrame`s marked as `ExclusivePage` are newly created through the COW mechanism
    /// and do not require further COW operations.
    ExclusivePage,
}

impl From<VmoMark> for XMark {
    fn from(val: VmoMark) -> Self {
        match val {
            VmoMark::CowVmo => XMark::Mark0,
            VmoMark::ExclusivePage => XMark::Mark1,
        }
    }
}

/// `Pages` is the struct that manages the `VmFrame`s stored in `Vmo_`.
pub(super) enum Pages {
    /// `Pages` that cannot be resized. This kind of `Pages` will have a constant size.
    Nonresizable(Arc<Mutex<XArray<VmFrame, VmoMark>>>, usize),
    /// `Pages` that can be resized and have a variable size, and such `Pages` cannot
    /// be shared between different VMOs.
    Resizable(Mutex<(XArray<VmFrame, VmoMark>, usize)>),
}

impl Pages {
    fn with<R, F>(&self, func: F) -> R
    where
        F: FnOnce(&mut XArray<VmFrame, VmoMark>, usize) -> R,
    {
        match self {
            Self::Nonresizable(pages, size) => func(&mut pages.lock(), *size),
            Self::Resizable(pages) => {
                let mut lock = pages.lock();
                let size = lock.1;
                func(&mut lock.0, size)
            }
        }
    }
}

/// `Vmo_` is the structure that actually manages the content of VMO.
/// Broadly speaking, there are two types of VMO:
/// 1. File-backed VMO: the VMO backed by a file and resides in the `PageCache`,
/// which includes a pager to provide it with actual pages.
/// 2. Anonymous VMO: the VMO without a file backup, which does not have a pager.
pub(super) struct Vmo_ {
    pager: Option<Arc<dyn Pager>>,
    /// Flags
    flags: VmoFlags,
    /// The offset of the range of pages corresponding to the VMO within `pages`.
    page_idx_offset: usize,
    /// The virtual pages where the VMO resides.
    pages: Pages,
}

fn clone_page(page: &VmFrame) -> Result<VmFrame> {
    let new_page = VmAllocOptions::new(1).alloc_single()?;
    new_page.copy_from_frame(page);
    Ok(new_page)
}

impl Vmo_ {
    /// Prepare a new `VmFrame` for the target index in pages, returning the new page as well as
    /// whether this page needs to be marked as exclusive.
    ///
    /// Based on the type of VMO and the impending operation on the prepared page, there are 3 conditions:
    /// 1. For an Anonymous VMO, provide a new page directly. If the VMO requires copy-on-write (COW),
    ///    the prepared page can be directly set to exclusive.
    /// 2. For a File-backed VMO that does not need to trigger the COW mechanism,
    ///    obtain a page from the pager directly without the need to be set as exclusive.
    /// 3. For a File-backed VMO that requires triggering the COW mechanism, obtain a page
    ///    from the pager and then copy it. This page can be set as exclusive.
    fn prepare_page(
        &self,
        page_idx: usize,
        is_cow_vmo: bool,
        will_write: bool,
    ) -> Result<(VmFrame, bool)> {
        let (page, should_mark_exclusive) = match &self.pager {
            None => {
                // Condition 1. The new anonymous page only need to be marked as `ExclusivePage`
                // when current VMO is a cow VMO, otherwise this mark is meaningless.
                (VmAllocOptions::new(1).alloc_single()?, is_cow_vmo)
            }
            Some(pager) => {
                let page = pager.commit_page(page_idx)?;
                // The prerequisite for triggering the COW mechanism here is that the current
                // VMO requires COW and the prepared page is about to undergo a write operation.
                // At this point, the `VmFrame` obtained from the pager needs to be cloned to
                // avoid subsequent modifications affecting the content of the `VmFrame` in the pager.
                let trigger_cow = is_cow_vmo && will_write;
                if trigger_cow {
                    // Condition 3.
                    (clone_page(&page)?, true)
                } else {
                    // Condition 2.
                    (page, false)
                }
            }
        };
        Ok((page, should_mark_exclusive))
    }

    fn commit_with_cursor(
        &self,
        cursor: &mut CursorMut<'_, VmFrame, VmoMark>,
        is_cow_vmo: bool,
        will_write: bool,
    ) -> Result<VmFrame> {
        let (new_page, is_exclusive) = {
            let is_exclusive = cursor.is_marked(VmoMark::ExclusivePage);
            if let Some(committed_page) = cursor.load() {
                // The necessary and sufficient condition for triggering the COW mechanism is that
                // the current VMO requires copy-on-write, there is an impending write operation to the page,
                // and the page is not exclusive.
                let trigger_cow = is_cow_vmo && will_write && !is_exclusive;
                if !trigger_cow {
                    // Fast path: return the page directly.
                    return Ok(committed_page.clone());
                }

                (clone_page(&committed_page)?, true)
            } else {
                self.prepare_page(cursor.index() as usize, is_cow_vmo, will_write)?
            }
        };

        cursor.store(new_page.clone());
        if is_exclusive {
            cursor.set_mark(VmoMark::ExclusivePage).unwrap();
        }
        Ok(new_page)
    }

    /// Commit the page corresponding to the target offset in the VMO and return that page.
    /// If the current offset has already been committed, the page will be returned directly.
    /// During the commit process, the Copy-On-Write (COW) mechanism may be triggered depending on the circumstances.
    pub fn commit_page(&self, offset: usize, will_write: bool) -> Result<VmFrame> {
        let page_idx = offset / PAGE_SIZE + self.page_idx_offset;
        self.pages.with(|pages, size| {
            let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
            let mut cursor = pages.cursor_mut(page_idx as u64);
            self.commit_with_cursor(&mut cursor, is_cow_vmo, will_write)
        })
    }

    /// Decommit the page corresponding to the target offset in the VMO.
    fn decommit_page(&mut self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE + self.page_idx_offset;
        self.pages.with(|pages, size| {
            let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
            let mut cursor = pages.cursor_mut(page_idx as u64);
            if cursor.remove().is_some()
                && let Some(pager) = &self.pager
                && !is_cow_vmo
            {
                pager.decommit_page(page_idx)?;
            }
            Ok(())
        })
    }

    /// Commit a range of pages in the VMO, returns the pages in this range.
    pub fn commit(&self, range: Range<usize>, will_write: bool) -> Result<VmFrameVec> {
        self.pages.with(|pages, size| {
            if range.end > size {
                return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
            }

            let raw_page_idx_range = get_page_idx_range(&range);
            let page_idx_range = (raw_page_idx_range.start + self.page_idx_offset)
                ..(raw_page_idx_range.end + self.page_idx_offset);
            let mut frames = VmFrameVec::new_with_capacity(page_idx_range.len());

            let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
            let mut cursor = pages.cursor_mut(page_idx_range.start as u64);
            for page_idx in page_idx_range {
                let committed_page =
                    self.commit_with_cursor(&mut cursor, is_cow_vmo, will_write)?;
                frames.push(committed_page);
                cursor.next();
            }
            Ok(frames)
        })
    }

    /// Decommit a range of pages in the VMO.
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        self.pages.with(|pages, size| {
            self.decommit_pages(pages, range)?;
            Ok(())
        })
    }

    /// Read the specified amount of buffer content starting from the target offset in the VMO.
    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let read_len = buf.len();
        let read_range = offset..(offset + read_len);
        let frames = self.commit(read_range, false)?;
        let read_offset = offset % PAGE_SIZE;
        Ok(frames.read_bytes(read_offset, buf)?)
    }

    /// Write the specified amount of buffer content starting from the target offset in the VMO.
    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let write_len = buf.len();
        let write_range = offset..(offset + write_len);
        let frames = self.commit(write_range.clone(), true)?;
        let write_offset = offset % PAGE_SIZE;
        frames.write_bytes(write_offset, buf)?;
        let is_cow_vmo = self.is_cow_vmo();
        if let Some(pager) = &self.pager
            && !is_cow_vmo
        {
            let raw_page_idx_range = get_page_idx_range(&write_range);
            let page_idx_range = (raw_page_idx_range.start + self.page_idx_offset)
                ..(raw_page_idx_range.end + self.page_idx_offset);
            for page_idx in page_idx_range {
                pager.update_page(page_idx)?;
            }
        }
        Ok(())
    }

    /// Clear the target range in current VMO.
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        let buffer = vec![0u8; range.end - range.start];
        self.write_bytes(range.start, &buffer)?;
        Ok(())
    }

    /// Return the size of current VMO.
    pub fn size(&self) -> usize {
        self.pages.with(|pages, size| size)
    }

    /// Return the page index offset of current VMO in corresponding pages.
    pub fn page_idx_offset(&self) -> usize {
        self.page_idx_offset
    }

    /// Clone the current `pages` to the child VMO.
    ///
    /// Depending on the type of the VMO and the child, there are 4 conditions:
    /// 1. For a slice child, directly share the current `pages` with that child.
    /// 2. For a COW child, and the current VMO requires COW, it is necessary to clear the
    ///    ExclusivePage mark in the current `pages` and clone a new `pages` to the child.
    /// 3. For a COW child, where the current VMO does not require COW and is a File-backed VMO.
    ///    In this case, a new `pages` needs to be cloned to the child, and the child's `pages`
    ///    require COW. The current `pages` do not need COW as they need to remain consistent with the pager.
    /// 4. For a COW child, where the current VMO does not require COW and is an Anonymous VMO.
    ///    In this case, a new `pages` needs to be cloned to the child, and both the current `pages` and
    ///    the child's `pages` require COW.
    pub fn clone_pages_for_child(
        &self,
        child_type: ChildType,
        child_flags: VmoFlags,
        range: &Range<usize>,
    ) -> Result<Pages> {
        let child_vmo_start = range.start;
        let child_vmo_end = range.end;
        debug_assert!(child_vmo_start % PAGE_SIZE == 0);
        debug_assert!(child_vmo_end % PAGE_SIZE == 0);
        if child_vmo_start % PAGE_SIZE != 0 || child_vmo_end % PAGE_SIZE != 0 {
            return_errno_with_message!(Errno::EINVAL, "VMO range does not aligned with PAGE_SIZE");
        }

        match child_type {
            ChildType::Slice => {
                if child_flags.contains(VmoFlags::RESIZABLE) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "a slice child VMO cannot be resizable"
                    );
                }

                let Pages::Nonresizable(ref pages, size) = self.pages else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "a resizable VMO cannot have a slice child"
                    );
                };

                // A slice child should be inside parent VMO's range
                debug_assert!(child_vmo_end <= size);
                if child_vmo_end > size {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "a slice child VMO cannot exceed its parent VMO's size"
                    );
                }
                // Condition 1.
                Ok(Pages::Nonresizable(pages.clone(), range.len()))
            }
            ChildType::Cow => {
                let new_pages = self.pages.with(|pages, size| {
                    // A Copy-on-Write child should intersect with parent VMO
                    debug_assert!(child_vmo_start <= size);
                    if child_vmo_start > size {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "a COW VMO should overlap with its parent"
                        );
                    }

                    let self_is_cow = pages.is_marked(VmoMark::CowVmo);
                    if self_is_cow {
                        // Condition 2.
                        pages.unset_mark_all(VmoMark::ExclusivePage);
                        return Ok(pages.clone());
                    }

                    if self.pager.is_some() {
                        // Condition 3.
                        let mut cloned_pages = pages.clone();
                        cloned_pages.set_mark(VmoMark::CowVmo);
                        return Ok(cloned_pages);
                    }

                    // Condition 4.
                    pages.set_mark(VmoMark::CowVmo);
                    Ok(pages.clone())
                })?;
                if child_flags.contains(VmoFlags::RESIZABLE) {
                    Ok(Pages::Resizable(Mutex::new((new_pages, range.len()))))
                } else {
                    Ok(Pages::Nonresizable(
                        Arc::new(Mutex::new(new_pages)),
                        range.len(),
                    ))
                }
            }
        }
    }

    /// Resize current VMO to target size.
    pub fn resize(&self, new_size: usize) -> Result<()> {
        assert!(self.flags.contains(VmoFlags::RESIZABLE));
        let new_size = new_size.align_up(PAGE_SIZE);

        let Pages::Resizable(ref pages) = self.pages else {
            return_errno_with_message!(Errno::EINVAL, "current VMO is not resizable");
        };

        let mut lock = pages.lock();
        let old_size = lock.1;
        if new_size == old_size {
            return Ok(());
        }
        if new_size < old_size {
            self.decommit_pages(&mut lock.0, new_size..old_size)?;
        }
        lock.1 = new_size;
        Ok(())
    }

    fn decommit_pages(
        &self,
        pages: &mut XArray<VmFrame, VmoMark>,
        range: Range<usize>,
    ) -> Result<()> {
        let raw_page_idx_range = get_page_idx_range(&range);
        let page_idx_range = (raw_page_idx_range.start + self.page_idx_offset)
            ..(raw_page_idx_range.end + self.page_idx_offset);
        let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
        let mut cursor = pages.cursor_mut(page_idx_range.start as u64);
        for page_idx in page_idx_range {
            if cursor.remove().is_some()
                && let Some(pager) = &self.pager
                && !is_cow_vmo
            {
                pager.decommit_page(page_idx)?;
            }
            cursor.next();
        }
        Ok(())
    }

    /// Determine whether a page is committed.
    pub fn is_page_committed(&self, page_idx: usize) -> bool {
        self.pages.with(|pages, size| {
            pages
                .load((page_idx + self.page_idx_offset) as u64)
                .is_some()
        })
    }

    /// Return the flags of current VMO.
    pub fn flags(&self) -> VmoFlags {
        self.flags
    }

    /// Determine whether the VMO is need COW mechanism.
    pub fn is_cow_vmo(&self) -> bool {
        self.pages
            .with(|pages, size| pages.is_marked(VmoMark::CowVmo))
    }
}

impl<R> Vmo<R> {
    /// Returns the size (in bytes) of a VMO.
    pub fn size(&self) -> usize {
        self.0.size()
    }

    /// Returns the flags of a VMO.
    pub fn flags(&self) -> VmoFlags {
        self.0.flags()
    }

    /// return whether a page is already committed
    pub fn is_page_committed(&self, page_idx: usize) -> bool {
        self.0.is_page_committed(page_idx)
    }

    pub fn get_committed_frame(&self, page_idx: usize, write_page: bool) -> Result<VmFrame> {
        self.0.commit_page(page_idx * PAGE_SIZE, write_page)
    }

    pub fn is_cow_vmo(&self) -> bool {
        self.0.is_cow_vmo()
    }
}

/// get the page index range that contains the offset range of vmo
pub fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}
