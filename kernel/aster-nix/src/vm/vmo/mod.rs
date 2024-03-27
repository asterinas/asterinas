// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Objects (VMOs).

use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use aster_frame::{
    collection::xarray::{XArray, XMark},
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

// We implement this trait for Vmo, so we can use functions on type like Vmo<R> without trait bounds.
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

/// Marks used for the XArray in `Vmo_`.
#[derive(Copy, Clone)]
pub(super) enum VmoMark {
    /// Marks used for the Vmo's `pages` which is managed by XArray.
    /// The Vmo whose `pages` is marked as `CowVmo` may require a Copy-On-Write (COW) operation
    /// when performing a write action.
    CowVmo,
    /// Marks used for the `VmFrame` stored within the pages marked as `CowVmo`,
    /// VmFrames marked as `ExclusivePage` are newly created through the COW mechanism and do not require further COW operations.
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

/// `Vmo_` is the structure that actually manages the content of Vmo.
/// Broadly speaking, there are two types of Vmo:
/// 1. File-backed Vmo: the Vmo backed by a file and resides in the `PageCache`,
/// which includes a pager to provide it with actual pages.
/// 2. Anonymous Vmo: the Vmo without a file backup, which does not have a pager.
pub(super) struct Vmo_ {
    pager: Option<Arc<dyn Pager>>,
    /// Flags
    flags: VmoFlags,
    /// The offset of the range of pages corresponding to the vmo within `pages`.
    page_idx_offset: usize,
    /// The contiguous pages where the Vmo resides. Pages are managed by XArray,
    /// and can be shared among slice children.
    pages: Arc<Mutex<XArray<VmFrame, VmoMark>>>,
    /// The size of current `Vmo_`.
    size: AtomicUsize,
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
    /// Based on the type of `Vmo` and the impending operation on the prepared page, there are 3 conditions:
    /// 1. For an Anonymous `Vmo`, provide a new page directly. If the Vmo requires copy-on-write (COW),
    ///    the prepared page can be directly set to exclusive.
    /// 2. For a File-backed `Vmo` that does not need to trigger the COW mechanism,
    ///    obtain a page from the pager directly without the need to be set as exclusive.
    /// 3. For a File-backed `Vmo` that requires triggering the COW mechanism, obtain a page
    ///    from the pager and then copy it. This page can be set as exclusive.
    fn prepare_page(
        &self,
        page_idx: usize,
        is_cow_vmo: bool,
        will_write: bool,
    ) -> Result<(VmFrame, bool)> {
        let (page, should_mark_exclusive) = match &self.pager {
            None => {
                // Condition 1.
                (VmAllocOptions::new(1).alloc_single()?, is_cow_vmo)
            }
            Some(pager) => {
                let page = pager.commit_page(page_idx)?;
                // The prerequisite for triggering the COW mechanism here is that the current
                // Vmo requires COW and the prepared page is about to undergo a write operation.
                // At this point, the VmFrame obtained from the pager needs to be cloned to
                // avoid subsequent modifications affecting the content of the VmFrame in the pager.
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

    /// Commit the page corresponding to the target offset in the Vmo and return that page.
    /// If the current offset has already been committed, the page will be returned directly.
    /// During the commit process, the Copy-On-Write (COW) mechanism may be triggered depending on the circumstances.
    pub fn commit_page(&self, offset: usize, will_write: bool) -> Result<VmFrame> {
        let page_idx = offset / PAGE_SIZE + self.page_idx_offset;
        let mut pages = self.pages.lock();
        let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
        let mut cursor = pages.cursor_mut(page_idx as u64);
        let (new_page, is_exclusive) = {
            let is_exclusive = cursor.is_marked(VmoMark::ExclusivePage);
            if let Some(committed_page) = cursor.load() {
                // The necessary and sufficient condition for triggering the COW mechanism is that
                // the current Vmo requires copy-on-write, there is an impending write operation to the page,
                // and the page is not exclusive.
                let trigger_cow = is_cow_vmo && will_write && !is_exclusive;
                if !trigger_cow {
                    // Fast path: return the page directly.
                    return Ok(committed_page.clone());
                }

                (clone_page(&committed_page)?, true)
            } else {
                self.prepare_page(page_idx, is_cow_vmo, will_write)?
            }
        };

        cursor.store(new_page.clone());
        if is_exclusive {
            cursor.set_mark(VmoMark::ExclusivePage).unwrap();
        }
        Ok(new_page)
    }

    /// Decommit the page corresponding to the target offset in the Vmo.
    fn decommit_page(&mut self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE + self.page_idx_offset;
        let mut pages = self.pages.lock();
        let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
        let mut cursor = pages.cursor_mut(page_idx as u64);
        if cursor.remove().is_some()
            && let Some(pager) = &self.pager
            && !is_cow_vmo
        {
            pager.decommit_page(page_idx)?;
        }
        Ok(())
    }

    /// Commit a range of pages in the Vmo, , returns the pages in this range.
    pub fn commit(&self, range: Range<usize>, will_write: bool) -> Result<VmFrameVec> {
        let mut pages = self.pages.lock();
        if range.end > self.size() {
            return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
        }

        let raw_page_idx_range = get_page_idx_range(&range);
        let page_idx_range = (raw_page_idx_range.start + self.page_idx_offset)
            ..(raw_page_idx_range.end + self.page_idx_offset);
        let mut frames = VmFrameVec::new_with_capacity(page_idx_range.len());

        let is_cow_vmo = pages.is_marked(VmoMark::CowVmo);
        let mut cursor = pages.cursor_mut(page_idx_range.start as u64);
        for page_idx in page_idx_range {
            let committed_page = {
                let is_exclusive = cursor.is_marked(VmoMark::ExclusivePage);
                // The page has been committed, load the page directly or load it after triggering COW.
                if let Some(committed_page) = cursor.load() {
                    // The necessary and sufficient condition for triggering the COW mechanism is that
                    // the current Vmo requires copy-on-write, there is an impending write operation to the page,
                    // and the page is not exclusive.
                    let trigger_cow = is_cow_vmo && will_write && !is_exclusive;
                    if !trigger_cow {
                        committed_page.clone()
                    } else {
                        let new_page = clone_page(&committed_page)?;
                        cursor.store(new_page.clone());
                        cursor.set_mark(VmoMark::ExclusivePage).unwrap();
                        new_page
                    }
                // The page has not been committed.
                } else {
                    let (new_page, is_exclusive) =
                        self.prepare_page(page_idx, is_cow_vmo, will_write)?;
                    cursor.store(new_page.clone());
                    if is_exclusive {
                        cursor.set_mark(VmoMark::ExclusivePage).unwrap();
                    }
                    new_page
                }
            };
            frames.push(committed_page);
            cursor.next();
        }
        Ok(frames)
    }

    /// Decommit a range of pages in the Vmo.
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        let pages = self.pages.lock();
        self.decommit_pages(pages, range)
    }

    /// Read the specified amount of buffer content starting from the target offset in the Vmo.
    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let read_len = buf.len();
        let read_range = offset..(offset + read_len);
        let frames = self.commit(read_range, false)?;
        let read_offset = offset % PAGE_SIZE;
        Ok(frames.read_bytes(read_offset, buf)?)
    }

    /// Write the specified amount of buffer content starting from the target offset in the Vmo.
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

    /// Clear the target range in current Vmo.
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        let buffer = vec![0u8; range.end - range.start];
        self.write_bytes(range.start, &buffer)?;
        Ok(())
    }

    /// Return the size of current Vmo.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Return the page index offset of current Vmo in corresponding pages.
    pub fn page_idx_offset(&self) -> usize {
        self.page_idx_offset
    }

    /// Clone the current `pages` to the child Vmo.
    ///
    /// Depending on the type of the Vmo and the child, there are 4 conditions:
    /// 1. For a slice child, directly share the current `pages` with that child.
    /// 2. For a COW child, and the current Vmo requires COW, it is necessary to clear the
    ///    ExclusivePage mark in the current `pages` and clone a new `pages` to the child.
    /// 3. For a COW child, where the current Vmo does not require COW and is a File-backed Vmo.
    ///    In this case, a new `pages` needs to be cloned to the child, and the child's `pages`
    ///    require COW. The current `pages` do not need COW as they need to remain consistent with the pager.
    /// 4. For a COW child, where the current Vmo does not require COW and is an Anonymous Vmo.
    ///    In this case, a new `pages` needs to be cloned to the child, and both the current `pages` and
    ///    the child's `pages` require COW.
    pub fn clone_pages_for_child(
        &self,
        child_type: ChildType,
        range: &Range<usize>,
    ) -> Result<Arc<Mutex<XArray<VmFrame, VmoMark>>>> {
        let child_vmo_start = range.start;
        let child_vmo_end = range.end;
        debug_assert!(child_vmo_start % PAGE_SIZE == 0);
        debug_assert!(child_vmo_end % PAGE_SIZE == 0);
        if child_vmo_start % PAGE_SIZE != 0 || child_vmo_end % PAGE_SIZE != 0 {
            return_errno_with_message!(Errno::EINVAL, "vmo range does not aligned with PAGE_SIZE");
        }

        match child_type {
            ChildType::Slice => {
                let parent_vmo_size = self.size();
                // A slice child should be inside parent vmo's range
                debug_assert!(child_vmo_end <= parent_vmo_size);
                if child_vmo_end > parent_vmo_size {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "slice child vmo cannot exceed parent vmo's size"
                    );
                }
                // Condition 1.
                Ok(self.pages.clone())
            }
            ChildType::Cow => {
                let mut pages = self.pages.lock();
                let parent_vmo_size = self.size();
                // A copy on Write child should intersect with parent vmo
                debug_assert!(child_vmo_start <= parent_vmo_size);
                if child_vmo_start > parent_vmo_size {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "COW vmo should overlap with its parent"
                    );
                }
                let self_is_cow = pages.is_marked(VmoMark::CowVmo);

                if self_is_cow {
                    // Condition 2.
                    pages.unset_mark_all(VmoMark::ExclusivePage);
                    return Ok(Arc::new(Mutex::new(pages.clone())));
                }

                if self.pager.is_some() {
                    // Condition 3.
                    let mut cloned_pages = pages.clone();
                    cloned_pages.set_mark(VmoMark::CowVmo);
                    return Ok(Arc::new(Mutex::new(cloned_pages)));
                }

                // Condition 4.
                pages.set_mark(VmoMark::CowVmo);
                Ok(Arc::new(Mutex::new(pages.clone())))
            }
        }
    }

    /// Resize current Vmo to target size.
    pub fn resize(&self, new_size: usize) -> Result<()> {
        assert!(self.flags.contains(VmoFlags::RESIZABLE));
        let new_size = new_size.align_up(PAGE_SIZE);

        let pages = self.pages.lock();
        let old_size = self.size();
        if new_size == old_size {
            return Ok(());
        }
        if new_size < old_size {
            self.decommit_pages(pages, new_size..old_size)?;
        }
        self.size.store(new_size, Ordering::Release);

        Ok(())
    }

    fn decommit_pages(
        &self,
        mut pages: MutexGuard<XArray<VmFrame, VmoMark>>,
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
        self.pages
            .lock()
            .load((page_idx + self.page_idx_offset) as u64)
            .is_some()
    }

    /// Return the flags of current Vmo.
    pub fn flags(&self) -> VmoFlags {
        self.flags
    }

    /// Determine whether the Vmo is need COW mechanism.
    pub fn is_cow_vmo(&self) -> bool {
        self.pages.lock().is_marked(VmoMark::CowVmo)
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
