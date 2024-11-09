// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

//! Virtual Memory Objects (VMOs).

use core::ops::Range;

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::{
    collections::xarray::{CursorMut, XArray},
    mm::{Frame, FrameAllocOptions, VmReader, VmWriter},
};

use crate::prelude::*;

mod dyn_cap;
mod options;
mod pager;
mod static_cap;

pub use options::VmoOptions;
pub use pager::Pager;

/// Virtual Memory Objects (VMOs) are a type of capability that represents a
/// range of memory pages.
///
/// # Features
///
///  * **I/O interface.** A VMO provides read and write methods to access the
///    memory pages that it contain.
///  * **On-demand paging.** The memory pages of a VMO (except for _contiguous_
///    VMOs) are allocated lazily when the page is first accessed.
///  * **Access control.** As capabilities, VMOs restrict the
///    accessible range of memory and the allowed I/O operations.
///  * **Device driver support.** If specified upon creation, VMOs will be
///    backed by physically contiguous memory pages starting at a target address.
///  * **File system support.** By default, a VMO's memory pages are initially
///    all zeros. But if a VMO is attached to a pager (`Pager`) upon creation,
///    then its memory pages will be populated by the pager.
///    With this pager mechanism, file systems can easily implement page caches
///    with VMOs by attaching the VMOs to pagers backed by inodes.
///
/// # Capabilities
///
/// As a capability, each VMO is associated with a set of access rights,
/// whose semantics are explained below.
///
///  * The Dup right allows duplicating a VMO and creating children out of
///    a VMO.
///  * The Read, Write, Exec rights allow creating memory mappings with
///    readable, writable, and executable access permissions, respectively.
///  * The Read and Write rights allow the VMO to be read from and written to
///    directly.
///  * The Write right allows resizing a resizable VMO.
///
/// VMOs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmo<Rights>`) and the static one (`Vmo<R: TRights>).
///
/// # Examples
///
/// For creating root VMOs, see [`VmoOptions`].
///
/// # Implementation
///
/// `Vmo` provides high-level APIs for address space management by wrapping
/// around its low-level counterpart [`ostd::mm::Frame`].
/// Compared with `Frame`,
/// `Vmo` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
#[derive(Debug)]
pub struct Vmo<R = Rights>(pub(super) Arc<Vmo_>, R);

/// Functions exist both for static capbility and dynamic capability
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
// FIXME: This requires the incomplete feature specialization, which should be fixed further.
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

/// `Pages` is the struct that manages the `Frame`s stored in `Vmo_`.
pub(super) enum Pages {
    /// `Pages` that cannot be resized. This kind of `Pages` will have a constant size.
    Nonresizable(Mutex<XArray<Frame>>, usize),
    /// `Pages` that can be resized and have a variable size.
    Resizable(Mutex<(XArray<Frame>, usize)>),
}

impl Clone for Pages {
    fn clone(&self) -> Self {
        match self {
            Self::Nonresizable(_, _) => {
                self.with(|pages, size| Self::Nonresizable(Mutex::new(pages.clone()), size))
            }
            Self::Resizable(_) => {
                self.with(|pages, size| Self::Resizable(Mutex::new((pages.clone(), size))))
            }
        }
    }
}

impl Pages {
    fn with<R, F>(&self, func: F) -> R
    where
        F: FnOnce(&mut XArray<Frame>, usize) -> R,
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
///    which includes a pager to provide it with actual pages.
/// 2. Anonymous VMO: the VMO without a file backup, which does not have a pager.
#[derive(Clone)]
pub(super) struct Vmo_ {
    pager: Option<Arc<dyn Pager>>,
    /// Flags
    flags: VmoFlags,
    /// The virtual pages where the VMO resides.
    pages: Pages,
}

impl Debug for Vmo_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vmo_")
            .field("flags", &self.flags)
            .field("size", &self.size())
            .finish()
    }
}

bitflags! {
    /// Commit Flags.
    pub struct CommitFlags: u8 {
        /// Set this flag if the page will be completely overwritten.
        /// This flag contains the WILL_WRITE flag.
        const WILL_OVERWRITE = 1;
    }
}

impl CommitFlags {
    pub fn will_overwrite(&self) -> bool {
        self.contains(Self::WILL_OVERWRITE)
    }
}

impl Vmo_ {
    /// Prepares a new `Frame` for the target index in pages, returns this new frame.
    fn prepare_page(&self, page_idx: usize) -> Result<Frame> {
        match &self.pager {
            None => Ok(FrameAllocOptions::new(1).alloc_single()?),
            Some(pager) => pager.commit_page(page_idx),
        }
    }

    /// Prepares a new `Frame` for the target index in the VMO, returns this new frame.
    fn prepare_overwrite(&self, page_idx: usize) -> Result<Frame> {
        if let Some(pager) = &self.pager {
            pager.commit_overwrite(page_idx)
        } else {
            Ok(FrameAllocOptions::new(1).alloc_single()?)
        }
    }

    fn commit_with_cursor(
        &self,
        cursor: &mut CursorMut<'_, Frame>,
        commit_flags: CommitFlags,
    ) -> Result<Frame> {
        let new_page = {
            if let Some(committed_page) = cursor.load() {
                // Fast path: return the page directly.
                return Ok(committed_page.clone());
            } else if commit_flags.will_overwrite() {
                // In this case, the page will be completely overwritten.
                self.prepare_overwrite(cursor.index() as usize)?
            } else {
                self.prepare_page(cursor.index() as usize)?
            }
        };

        cursor.store(new_page.clone());
        Ok(new_page)
    }

    /// Commits the page corresponding to the target offset in the VMO and return that page.
    /// If the current offset has already been committed, the page will be returned directly.
    pub fn commit_page(&self, offset: usize) -> Result<Frame> {
        let page_idx = offset / PAGE_SIZE;
        self.pages.with(|pages, size| {
            if offset >= size {
                return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
            }
            let mut cursor = pages.cursor_mut(page_idx as u64);
            self.commit_with_cursor(&mut cursor, CommitFlags::empty())
        })
    }

    /// Decommits the page corresponding to the target offset in the VMO.
    fn decommit_page(&mut self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        self.pages.with(|pages, size| {
            if offset >= size {
                return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
            }
            let mut cursor = pages.cursor_mut(page_idx as u64);
            if cursor.remove().is_some()
                && let Some(pager) = &self.pager
            {
                pager.decommit_page(page_idx)?;
            }
            Ok(())
        })
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    pub fn operate_on_range<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        commit_flags: CommitFlags,
    ) -> Result<()>
    where
        F: FnMut(&mut dyn FnMut() -> Result<Frame>) -> Result<()>,
    {
        self.pages.with(|pages, size| {
            if range.end > size {
                return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
            }

            let page_idx_range = get_page_idx_range(range);
            let mut cursor = pages.cursor_mut(page_idx_range.start as u64);
            for page_idx in page_idx_range {
                let mut commit_fn = || self.commit_with_cursor(&mut cursor, commit_flags);
                operate(&mut commit_fn)?;
                cursor.next();
            }
            Ok(())
        })
    }

    /// Decommits a range of pages in the VMO.
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        self.pages.with(|pages, size| {
            if range.end > size {
                return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
            }

            self.decommit_pages(pages, range)?;
            Ok(())
        })
    }

    /// Reads the specified amount of buffer content starting from the target offset in the VMO.
    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        let read_range = offset..(offset + read_len);
        let mut read_offset = offset % PAGE_SIZE;

        let read = move |commit_fn: &mut dyn FnMut() -> Result<Frame>| {
            let frame = commit_fn()?;
            frame.reader().skip(read_offset).read_fallible(writer)?;
            read_offset = 0;
            Ok(())
        };

        self.operate_on_range(&read_range, read, CommitFlags::empty())
    }

    /// Writes the specified amount of buffer content starting from the target offset in the VMO.
    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain();
        let write_range = offset..(offset + write_len);
        let mut write_offset = offset % PAGE_SIZE;

        let mut write = move |commit_fn: &mut dyn FnMut() -> Result<Frame>| {
            let frame = commit_fn()?;
            frame.writer().skip(write_offset).write_fallible(reader)?;
            write_offset = 0;
            Ok(())
        };

        if write_range.len() < PAGE_SIZE {
            self.operate_on_range(&write_range, write, CommitFlags::empty())?;
        } else {
            let temp = write_range.start + PAGE_SIZE - 1;
            let up_align_start = temp - temp % PAGE_SIZE;
            let down_align_end = write_range.end - write_range.end % PAGE_SIZE;
            if write_range.start != up_align_start {
                let head_range = write_range.start..up_align_start;
                self.operate_on_range(&head_range, &mut write, CommitFlags::empty())?;
            }
            if up_align_start != down_align_end {
                let mid_range = up_align_start..down_align_end;
                self.operate_on_range(&mid_range, &mut write, CommitFlags::WILL_OVERWRITE)?;
            }
            if down_align_end != write_range.end {
                let tail_range = down_align_end..write_range.end;
                self.operate_on_range(&tail_range, &mut write, CommitFlags::empty())?;
            }
        }

        if let Some(pager) = &self.pager {
            let page_idx_range = get_page_idx_range(&write_range);
            for page_idx in page_idx_range {
                pager.update_page(page_idx)?;
            }
        }
        Ok(())
    }

    /// Clears the target range in current VMO.
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        let buffer = vec![0u8; range.end - range.start];
        let mut reader = VmReader::from(buffer.as_slice()).to_fallible();
        self.write(range.start, &mut reader)?;
        Ok(())
    }

    /// Returns the size of current VMO.
    pub fn size(&self) -> usize {
        self.pages.with(|_, size| size)
    }

    /// Resizes current VMO to target size.
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

    fn decommit_pages(&self, pages: &mut XArray<Frame>, range: Range<usize>) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        let mut cursor = pages.cursor_mut(page_idx_range.start as u64);
        for page_idx in page_idx_range {
            if cursor.remove().is_some()
                && let Some(pager) = &self.pager
            {
                pager.decommit_page(page_idx)?;
            }
            cursor.next();
        }
        Ok(())
    }

    /// Determines whether a page is committed.
    pub fn is_page_committed(&self, page_idx: usize) -> bool {
        self.pages
            .with(|pages, _| pages.load(page_idx as u64).is_some())
    }

    /// Returns the flags of current VMO.
    pub fn flags(&self) -> VmoFlags {
        self.flags
    }

    fn replace(&self, page: Frame, page_idx: usize) -> Result<()> {
        self.pages.with(|pages, size| {
            if page_idx >= size / PAGE_SIZE {
                return_errno_with_message!(Errno::EINVAL, "the page index is outside of the vmo");
            }
            pages.store(page_idx as u64, page);
            Ok(())
        })
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
}

/// Gets the page index range that contains the offset range of VMO.
pub fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}
