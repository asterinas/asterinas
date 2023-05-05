//! Virtual Memory Objects (VMOs).

use core::ops::Range;

use crate::rights::Rights;
use align_ext::AlignExt;
use jinux_frame::vm::{VmAllocOptions, VmFrameVec, VmIo};

use crate::prelude::*;

mod dyn_cap;
mod options;
mod pager;
mod static_cap;

pub use options::{VmoChildOptions, VmoOptions};
pub use pager::Pager;

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
/// around its low-level counterpart `jinux_frame::vm::VmFrames`.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmoType {
    /// This vmo_ is created as a copy on write child
    CopyOnWriteChild,
    /// This vmo_ is created as a slice child
    SliceChild,
    /// This vmo_ is not created as a child of a parent vmo
    NotChild,
}

pub(super) struct Vmo_ {
    /// Flags
    flags: VmoFlags,
    /// VmoInner
    inner: Mutex<VmoInner>,
    /// Parent Vmo
    parent: Weak<Vmo_>,
    /// vmo type
    vmo_type: VmoType,
}

struct VmoInner {
    /// The backup pager
    pager: Option<Arc<dyn Pager>>,
    /// size, in bytes
    size: usize,
    /// The pages committed. The key is the page index, the value is the backup frame.
    committed_pages: BTreeMap<usize, VmFrameVec>,
    /// The pages from the parent that current vmo can access. The pages can only be inherited when create childs vmo.
    /// We store the page index range
    inherited_pages: InheritedPages,
}

/// Pages inherited from parent
struct InheritedPages {
    /// The page index range in child vmo. The pages inside these range are initially inherited from parent vmo.
    /// The range includes the start page, but not including the end page
    page_range: Range<usize>,
    /// The page index offset in parent vmo. That is to say, the page with index `idx` in child vmo corrsponds to
    /// page with index `idx + parent_page_idx_offset` in parent vmo
    parent_page_idx_offset: usize,
}

impl InheritedPages {
    pub fn new_empty() -> Self {
        Self {
            page_range: 0..0,
            parent_page_idx_offset: 0,
        }
    }

    pub fn new(page_range: Range<usize>, parent_page_idx_offset: usize) -> Self {
        Self {
            page_range,
            parent_page_idx_offset,
        }
    }

    fn contains_page(&self, page_idx: usize) -> bool {
        self.page_range.start <= page_idx && page_idx < self.page_range.end
    }

    fn parent_page_idx(&self, child_page_idx: usize) -> Option<usize> {
        if self.contains_page(child_page_idx) {
            Some(child_page_idx + self.parent_page_idx_offset)
        } else {
            None
        }
    }
}

impl Vmo_ {
    pub fn commit_page(&self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        let mut inner = self.inner.lock();
        if !inner.committed_pages.contains_key(&page_idx) {
            let frames = match &inner.pager {
                None => {
                    let vm_alloc_option = VmAllocOptions::new(1);
                    let frames = VmFrameVec::allocate(&vm_alloc_option)?;
                    frames.iter().for_each(|frame| frame.zero());
                    frames
                }
                Some(pager) => {
                    let frame = pager.commit_page(offset)?;
                    VmFrameVec::from_one_frame(frame)
                }
            };
            inner.committed_pages.insert(page_idx, frames);
        }
        Ok(())
    }

    pub fn decommit_page(&self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        let mut inner = self.inner.lock();
        if inner.committed_pages.contains_key(&page_idx) {
            inner.committed_pages.remove(&page_idx);
            if let Some(pager) = &inner.pager {
                pager.decommit_page(offset)?;
            }
        }
        Ok(())
    }

    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        for page_idx in page_idx_range {
            let offset = page_idx * PAGE_SIZE;
            self.commit_page(offset)?;
        }

        Ok(())
    }

    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        for page_idx in page_idx_range {
            let offset = page_idx * PAGE_SIZE;
            self.decommit_page(offset)?;
        }
        Ok(())
    }

    /// determine whether a page is commited
    pub fn page_commited(&self, page_idx: usize) -> bool {
        self.inner.lock().committed_pages.contains_key(&page_idx)
    }

    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let read_len = buf.len();
        debug_assert!(offset + read_len <= self.size());
        if offset + read_len > self.size() {
            return_errno_with_message!(Errno::EINVAL, "read range exceeds vmo size");
        }
        let read_range = offset..(offset + read_len);
        let frames = self.ensure_all_pages_exist(&read_range, false)?;
        let read_offset = offset % PAGE_SIZE;
        Ok(frames.read_bytes(read_offset, buf)?)
    }

    /// Ensure all pages inside range are backed up vm frames, returns the frames.
    fn ensure_all_pages_exist(&self, range: &Range<usize>, write_page: bool) -> Result<VmFrameVec> {
        let page_idx_range = get_page_idx_range(range);
        let mut frames = VmFrameVec::empty();
        for page_idx in page_idx_range {
            let mut page_frame = self.get_backup_frame(page_idx, write_page, true)?;
            frames.append(&mut page_frame)?;
        }
        Ok(frames)
    }

    /// Get the backup frame for a page. If commit_if_none is set, we will commit a new page for the page
    /// if the page does not have a backup frame.
    fn get_backup_frame(
        &self,
        page_idx: usize,
        write_page: bool,
        commit_if_none: bool,
    ) -> Result<VmFrameVec> {
        // if the page is already commit, return the committed page.
        if let Some(frames) = self.inner.lock().committed_pages.get(&page_idx) {
            return Ok(frames.clone());
        }

        match self.vmo_type {
            // if the vmo is not child, then commit new page
            VmoType::NotChild => {
                if commit_if_none {
                    self.commit_page(page_idx * PAGE_SIZE)?;
                    let frames = self
                        .inner
                        .lock()
                        .committed_pages
                        .get(&page_idx)
                        .unwrap()
                        .clone();
                    return Ok(frames);
                } else {
                    return_errno_with_message!(Errno::EINVAL, "backup frame does not exist");
                }
            }
            // if the vmo is slice child, we will request the frame from parent
            VmoType::SliceChild => {
                let inner = self.inner.lock();
                debug_assert!(inner.inherited_pages.contains_page(page_idx));
                if !inner.inherited_pages.contains_page(page_idx) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "page does not inherited from parent"
                    );
                }
                let parent = self.parent.upgrade().unwrap();
                let parent_page_idx = inner.inherited_pages.parent_page_idx(page_idx).unwrap();
                return parent.get_backup_frame(parent_page_idx, write_page, commit_if_none);
            }
            // If the vmo is copy on write
            VmoType::CopyOnWriteChild => {
                if write_page {
                    // write
                    // commit a new page
                    self.commit_page(page_idx * PAGE_SIZE)?;
                    let inner = self.inner.lock();
                    let frames = inner.committed_pages.get(&page_idx).unwrap().clone();
                    if let Some(parent_page_idx) = inner.inherited_pages.parent_page_idx(page_idx) {
                        // copy contents of parent to the frame
                        let mut tmp_buffer = Box::new([0u8; PAGE_SIZE]);
                        let parent = self.parent.upgrade().unwrap();
                        parent.read_bytes(parent_page_idx * PAGE_SIZE, &mut *tmp_buffer)?;
                        frames.write_bytes(0, &*tmp_buffer)?;
                    } else {
                        frames.zero();
                    }
                    return Ok(frames);
                } else {
                    // read
                    let parent_page_idx =
                        self.inner.lock().inherited_pages.parent_page_idx(page_idx);
                    if let Some(parent_page_idx) = parent_page_idx {
                        // If it's inherited from parent, we request the page from parent
                        let parent = self.parent.upgrade().unwrap();
                        return parent.get_backup_frame(
                            parent_page_idx,
                            write_page,
                            commit_if_none,
                        );
                    } else {
                        // Otherwise, we commit a new page
                        self.commit_page(page_idx * PAGE_SIZE)?;
                        let frames = self
                            .inner
                            .lock()
                            .committed_pages
                            .get(&page_idx)
                            .unwrap()
                            .clone();
                        // FIXME: should we zero the frames here?
                        frames.zero();
                        return Ok(frames);
                    }
                }
            }
        }
    }

    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let write_len = buf.len();
        debug_assert!(offset + write_len <= self.size());
        if offset + write_len > self.size() {
            return_errno_with_message!(Errno::EINVAL, "write range exceeds the vmo size");
        }

        let write_range = offset..(offset + write_len);
        let frames = self.ensure_all_pages_exist(&write_range, true)?;
        let write_offset = offset % PAGE_SIZE;
        frames.write_bytes(write_offset, buf)?;
        if let Some(pager) = &self.inner.lock().pager {
            let page_idx_range = get_page_idx_range(&write_range);
            for page_idx in page_idx_range {
                pager.update_page(page_idx * PAGE_SIZE)?;
            }
        }
        Ok(())
    }

    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        let buffer = vec![0u8; range.end - range.start];
        self.write_bytes(range.start, &buffer)
    }

    pub fn size(&self) -> usize {
        self.inner.lock().size
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        assert!(self.flags.contains(VmoFlags::RESIZABLE));
        let new_size = new_size.align_up(PAGE_SIZE);
        let old_size = self.size();
        if new_size == old_size {
            return Ok(());
        }

        if new_size < old_size {
            self.decommit(new_size..old_size)?;
            self.inner.lock().size = new_size;
        } else {
            self.inner.lock().size = new_size;
        }

        Ok(())
    }

    pub fn flags(&self) -> VmoFlags {
        self.flags.clone()
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
    pub fn has_backup_frame(&self, page_idx: usize) -> bool {
        if let Ok(_) = self.0.get_backup_frame(page_idx, false, false) {
            true
        } else {
            false
        }
    }

    pub fn get_backup_frame(
        &self,
        page_idx: usize,
        write_page: bool,
        commit_if_none: bool,
    ) -> Result<VmFrameVec> {
        self.0
            .get_backup_frame(page_idx, write_page, commit_if_none)
    }

    pub fn is_cow_child(&self) -> bool {
        self.0.vmo_type == VmoType::CopyOnWriteChild
    }
}

/// get the page index range that contains the offset range of vmo
pub fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}
