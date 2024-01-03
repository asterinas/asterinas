// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Objects (VMOs).

use core::ops::Range;

use align_ext::AlignExt;
use aster_frame::vm::{VmAllocOptions, VmFrame, VmFrameVec, VmIo};
use aster_rights::Rights;

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

pub(super) struct Vmo_ {
    /// Flags
    flags: VmoFlags,
    /// VmoInner
    inner: Mutex<VmoInner>,
}

struct VmoInner {
    pager: Option<Arc<dyn Pager>>,
    /// size, in bytes
    size: usize,
    /// The pages committed. The key is the page index, the value is the committed frame.
    committed_pages: BTreeMap<usize, VmFrame>,
    /// The pages from the parent that current vmo can access. The pages can only be inherited when create childs vmo.
    /// We store the page index range
    inherited_pages: Option<Vec<VmFrame>>,
    /// Whether the vmo is copy on write child.
    is_cow: bool,
}

impl VmoInner {
    fn commit_page(&mut self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        // Fast path: the page is already committed.
        if self.committed_pages.contains_key(&page_idx) {
            return Ok(());
        }
        let frame = match &self.pager {
            None => VmAllocOptions::new(1).alloc_single()?,
            Some(pager) => pager.commit_page(page_idx)?,
        };
        self.insert_frame(page_idx, frame);
        Ok(())
    }

    fn decommit_page(&mut self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        if self.committed_pages.remove(&page_idx).is_some() {
            if let Some(pager) = &self.pager {
                pager.decommit_page(page_idx)?;
            }
        }
        Ok(())
    }

    fn insert_frame(&mut self, page_idx: usize, frame: VmFrame) {
        debug_assert!(!self.committed_pages.contains_key(&page_idx));
        self.committed_pages.insert(page_idx, frame);
    }

    fn get_committed_frame(&mut self, page_idx: usize, write_page: bool) -> Result<VmFrame> {
        // if the page is already commit, return the committed page.
        if let Some(frames) = self.committed_pages.get(&page_idx) {
            return Ok(frames.clone());
        }

        // The vmo is not child
        if self.inherited_pages.is_none() {
            self.commit_page(page_idx * PAGE_SIZE)?;
            let frame = self.committed_pages.get(&page_idx).unwrap().clone();
            return Ok(frame);
        }

        let frame = self.get_inherited_frame_or_alloc(page_idx, write_page)?;

        if !self.should_share_frame_with_parent(write_page) {
            self.insert_frame(page_idx, frame.clone());
        }

        Ok(frame)
    }

    fn get_inherited_frame_or_alloc(&self, page_idx: usize, write_page: bool) -> Result<VmFrame> {
        let inherited_frames = self.inherited_pages.as_ref().unwrap();

        if page_idx >= inherited_frames.len() {
            if self.is_cow {
                return Ok(VmAllocOptions::new(1).alloc_single()?);
            }
            return_errno_with_message!(Errno::EINVAL, "the page is not inherited from parent");
        }

        let inherited_frame = inherited_frames.get(page_idx).unwrap().clone();

        if self.should_share_frame_with_parent(write_page) {
            return Ok(inherited_frame);
        }

        let frame = VmAllocOptions::new(1).alloc_single()?;
        frame.copy_from_frame(&inherited_frame);
        Ok(frame)
    }

    fn is_cow_child(&self) -> bool {
        self.is_cow
    }

    fn should_share_frame_with_parent(&self, write_page: bool) -> bool {
        !self.is_cow || !write_page
    }
}

impl Vmo_ {
    pub fn commit_page(&self, offset: usize) -> Result<()> {
        self.inner.lock().commit_page(offset)
    }

    pub fn decommit_page(&self, offset: usize) -> Result<()> {
        self.inner.lock().decommit_page(offset)
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
        let mut frames = VmFrameVec::new_with_capacity(page_idx_range.len());
        for page_idx in page_idx_range {
            let page_frame = self.get_committed_frame(page_idx, write_page)?;
            frames.push(page_frame);
        }
        Ok(frames)
    }

    /// Get the frame for a page. If commit_if_none is set, we will commit a new page for the page
    /// if the page is not committed.
    fn get_committed_frame(&self, page_idx: usize, write_page: bool) -> Result<VmFrame> {
        self.inner.lock().get_committed_frame(page_idx, write_page)
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
                pager.update_page(page_idx)?;
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
        self.flags
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
        self.0.page_commited(page_idx)
    }

    pub fn get_committed_frame(&self, page_idx: usize, write_page: bool) -> Result<VmFrame> {
        self.0.get_committed_frame(page_idx, write_page)
    }

    pub fn is_cow_child(&self) -> bool {
        self.0.inner.lock().is_cow_child()
    }
}

/// get the page index range that contains the offset range of vmo
pub fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}

pub(super) fn get_inherited_frames_from_parent(
    parent: Arc<Vmo_>,
    num_pages: usize,
    parent_page_idx_offset: usize,
    is_cow: bool,
) -> Vec<VmFrame> {
    let mut inherited_frames = Vec::with_capacity(num_pages);
    for page_idx in 0..num_pages {
        let parent_page_idx = page_idx + parent_page_idx_offset;
        let inherited_frame = parent
            .get_committed_frame(parent_page_idx, !is_cow)
            .unwrap();
        inherited_frames.push(inherited_frame);
    }
    inherited_frames
}
