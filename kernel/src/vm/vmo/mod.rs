// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(unused_variables)]

//! Virtual Memory Objects (VMOs).

use core::{
    ops::Range,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use ostd::{
    mm::{
        FrameAllocOptions, UFrame, VmIo, VmIoFill, VmReader, VmWriter, io_util::HasVmReaderWriter,
    },
    task::disable_preempt,
};
use xarray::{Cursor, LockedXArray, XArray};

use crate::prelude::*;

mod options;
mod pager;

pub use options::VmoOptions;
pub use pager::Pager;

/// Virtual Memory Objects (VMOs) are a type of capability that represents a
/// range of memory pages.
///
/// Broadly speaking, there are two types of VMO:
/// 1. File-backed VMO: the VMO backed by a file and resides in the page cache,
///    which includes a [`Pager`] to provide it with actual pages.
/// 2. Anonymous VMO: the VMO without a file backup, which does not have a `Pager`.
///
/// # Features
///
///  * **I/O interface.** A VMO provides read and write methods to access the
///    memory pages that it contain.
///  * **On-demand paging.** The memory pages of a VMO (except for _contiguous_
///    VMOs) are allocated lazily when the page is first accessed.
///  * **Device driver support.** If specified upon creation, VMOs will be
///    backed by physically contiguous memory pages starting at a target address.
///  * **File system support.** By default, a VMO's memory pages are initially
///    all zeros. But if a VMO is attached to a pager (`Pager`) upon creation,
///    then its memory pages will be populated by the pager.
///    With this pager mechanism, file systems can easily implement page caches
///    with VMOs by attaching the VMOs to pagers backed by inodes.
///
/// # Examples
///
/// For creating root VMOs, see [`VmoOptions`].
///
/// # Implementation
///
/// `Vmo` provides high-level APIs for address space management by wrapping
/// around its low-level counterpart [`ostd::mm::UFrame`].
/// Compared with `UFrame`,
/// `Vmo` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
pub struct Vmo {
    pager: Option<Arc<dyn Pager>>,
    /// Flags
    flags: VmoFlags,
    /// The virtual pages where the VMO resides.
    pages: XArray<UFrame>,
    /// The size of the VMO.
    ///
    /// Note: This size may not necessarily match the size of the `pages`, but it is
    /// required here that modifications to the size can only be made after locking
    /// the [`XArray`] in the `pages` field. Therefore, the size read after locking the
    /// `pages` will be the latest size.
    size: AtomicUsize,
    /// The status of writable mappings of the VMO.
    //
    // TODO: This field is used only by VMOs belonging to memfd (i.e., `MemfdInode`). But VMOs do
    // not have the knowledge to determine if they belong to memfd. We may want to enhance
    // `VmoOptions` to make VMOs aware of whether its writable mappings should be tracked.
    writable_mapping_status: WritableMappingStatus,
}

impl Debug for Vmo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vmo")
            .field("flags", &self.flags)
            .field("size", &self.size)
            .field("writable_mapping_status", &self.writable_mapping_status)
            .finish_non_exhaustive()
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

/// The error type used for commit operations of [`Vmo`].
#[derive(Debug)]
pub enum VmoCommitError {
    /// Represents a general error raised during the commit operation.
    Err(Error),
    /// Represents that the commit operation need to do I/O operation on the
    /// wrapped index.
    NeedIo(usize),
}

impl From<Error> for VmoCommitError {
    fn from(e: Error) -> Self {
        VmoCommitError::Err(e)
    }
}

impl From<ostd::Error> for VmoCommitError {
    fn from(e: ostd::Error) -> Self {
        Error::from(e).into()
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

impl Vmo {
    /// Prepares a new `UFrame` for the target index in pages, returns this new frame.
    ///
    /// This operation may involve I/O operations if the VMO is backed by a pager.
    fn prepare_page(&self, page_idx: usize, commit_flags: CommitFlags) -> Result<UFrame> {
        match &self.pager {
            None => Ok(FrameAllocOptions::new().alloc_frame()?.into()),
            Some(pager) => {
                if commit_flags.will_overwrite() {
                    pager.commit_overwrite(page_idx)
                } else {
                    pager.commit_page(page_idx)
                }
            }
        }
    }

    /// Commits a page at a specific page index.
    ///
    /// This method may involve I/O operations if the VMO needs to fetch a page from
    /// the underlying page cache.
    pub fn commit_on(&self, page_idx: usize, commit_flags: CommitFlags) -> Result<UFrame> {
        let new_page = self.prepare_page(page_idx, commit_flags)?;

        let mut locked_pages = self.pages.lock();
        if page_idx * PAGE_SIZE > self.size() {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            return Ok(page.clone());
        }

        cursor.store(new_page.clone());
        Ok(new_page)
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, UFrame>,
    ) -> core::result::Result<UFrame, VmoCommitError> {
        if let Some(committed_page) = cursor.load() {
            return Ok(committed_page.clone());
        }

        if let Some(pager) = &self.pager {
            // FIXME: Here `Vmo` treat all instructions in `pager` as I/O instructions
            // since it needs to take the inner `Mutex` lock and users also cannot hold a
            // `SpinLock` to do such instructions. This workaround may introduce some performance
            // issues. In the future we should solve the redundancy of `Vmo` and the pagecache
            // make sure return such error when really needing I/Os.
            return Err(VmoCommitError::NeedIo(cursor.index() as usize));
        }

        let frame = self.commit_on(cursor.index() as usize, CommitFlags::empty())?;
        Ok(frame)
    }

    /// Commits the page corresponding to the target offset in the VMO.
    ///
    /// If the commit operation needs to perform I/O, it will return a [`VmoCommitError::NeedIo`].
    pub fn try_commit_page(&self, offset: usize) -> core::result::Result<UFrame, VmoCommitError> {
        let page_idx = offset / PAGE_SIZE;
        if offset >= self.size() {
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EINVAL,
                "the offset is outside the VMO",
            )));
        }

        let guard = disable_preempt();
        let mut cursor = self.pages.cursor(&guard, page_idx as u64);
        self.try_commit_with_cursor(&mut cursor)
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    ///
    /// Once a commit operation needs to perform I/O, it will return a [`VmoCommitError::NeedIo`].
    pub fn try_operate_on_range<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
    ) -> core::result::Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
    {
        if range.end > self.size() {
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EINVAL,
                "operated range exceeds the vmo size",
            )));
        }

        let page_idx_range = get_page_idx_range(range);
        let guard = disable_preempt();
        let mut cursor = self.pages.cursor(&guard, page_idx_range.start as u64);
        for page_idx in page_idx_range {
            let mut commit_fn = || self.try_commit_with_cursor(&mut cursor);
            operate(&mut commit_fn)?;
            cursor.next();
        }
        Ok(())
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    ///
    /// This method may involve I/O operations if the VMO needs to fetch a page from
    /// the underlying page cache.
    fn operate_on_range<F>(
        &self,
        mut range: Range<usize>,
        mut operate: F,
        commit_flags: CommitFlags,
    ) -> Result<()>
    where
        F: FnMut(
            &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
    {
        'retry: loop {
            let res = self.try_operate_on_range(&range, &mut operate);
            match res {
                Ok(_) => return Ok(()),
                Err(VmoCommitError::Err(e)) => return Err(e),
                Err(VmoCommitError::NeedIo(index)) => {
                    self.commit_on(index, commit_flags)?;
                    range.start = index * PAGE_SIZE;
                    continue 'retry;
                }
            }
        }
    }

    /// Decommits a range of pages in the VMO.
    ///
    /// The range must be within the size of the VMO.
    ///
    /// The start and end addresses will be rounded down and up to page boundaries.
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        let locked_pages = self.pages.lock();
        if range.end > self.size() {
            return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
        }

        self.decommit_pages(locked_pages, range)?;
        Ok(())
    }

    /// Reads the specified amount of buffer content starting from the target offset in the VMO.
    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        let read_range = offset..(offset + read_len);
        let mut read_offset = offset % PAGE_SIZE;

        let read =
            move |commit_fn: &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>| {
                let frame = commit_fn()?;
                frame
                    .reader()
                    .skip(read_offset)
                    .read_fallible(writer)
                    .map_err(|e| VmoCommitError::from(e.0))?;
                read_offset = 0;
                Ok(())
            };

        self.operate_on_range(read_range, read, CommitFlags::empty())
    }

    /// Writes the specified amount of buffer content starting from the target offset in the VMO.
    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain();
        let write_range = offset..(offset + write_len);
        let mut write_offset = offset % PAGE_SIZE;
        let mut write =
            move |commit_fn: &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>| {
                let frame = commit_fn()?;
                frame
                    .writer()
                    .skip(write_offset)
                    .write_fallible(reader)
                    .map_err(|e| VmoCommitError::from(e.0))?;
                write_offset = 0;
                Ok(())
            };

        if write_range.len() < PAGE_SIZE {
            self.operate_on_range(write_range.clone(), write, CommitFlags::empty())?;
        } else {
            let temp = write_range.start + PAGE_SIZE - 1;
            let up_align_start = temp - temp % PAGE_SIZE;
            let down_align_end = write_range.end - write_range.end % PAGE_SIZE;
            if write_range.start != up_align_start {
                let head_range = write_range.start..up_align_start;
                self.operate_on_range(head_range, &mut write, CommitFlags::empty())?;
            }
            if up_align_start != down_align_end {
                let mid_range = up_align_start..down_align_end;
                self.operate_on_range(mid_range, &mut write, CommitFlags::WILL_OVERWRITE)?;
            }
            if down_align_end != write_range.end {
                let tail_range = down_align_end..write_range.end;
                self.operate_on_range(tail_range, &mut write, CommitFlags::empty())?;
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

    /// Clears the target range in current VMO by writing zeros.
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        let buffer = vec![0u8; range.end - range.start];
        let mut reader = VmReader::from(buffer.as_slice()).to_fallible();
        self.write(range.start, &mut reader)?;
        Ok(())
    }

    /// Returns the size of current VMO.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Resizes current VMO to target size.
    ///
    /// The VMO must be resizable.
    ///
    /// The new size will be rounded up to page boundaries.
    pub fn resize(&self, new_size: usize) -> Result<()> {
        assert!(self.flags.contains(VmoFlags::RESIZABLE));
        let new_size = new_size.align_up(PAGE_SIZE);

        let locked_pages = self.pages.lock();

        let old_size = self.size();
        if new_size == old_size {
            return Ok(());
        }

        self.size.store(new_size, Ordering::Release);

        if new_size < old_size {
            self.decommit_pages(locked_pages, new_size..old_size)?;
        }

        Ok(())
    }

    fn decommit_pages(
        &self,
        mut locked_pages: LockedXArray<UFrame>,
        range: Range<usize>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        let mut cursor = locked_pages.cursor_mut(page_idx_range.start as u64);

        let Some(pager) = &self.pager else {
            cursor.remove();
            while let Some(page_idx) = cursor.next_present()
                && page_idx < page_idx_range.end as u64
            {
                cursor.remove();
            }
            return Ok(());
        };

        let mut removed_page_idx = Vec::new();
        if cursor.remove().is_some() {
            removed_page_idx.push(page_idx_range.start);
        }
        while let Some(page_idx) = cursor.next_present()
            && page_idx < page_idx_range.end as u64
        {
            removed_page_idx.push(page_idx as usize);
            cursor.remove();
        }

        drop(locked_pages);

        for page_idx in removed_page_idx {
            pager.decommit_page(page_idx)?;
        }

        Ok(())
    }

    /// Returns the flags of current VMO.
    pub fn flags(&self) -> VmoFlags {
        self.flags
    }

    /// Replaces the page at the `page_idx` in the VMO with the input `page`.
    fn replace(&self, page: UFrame, page_idx: usize) -> Result<()> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size() / PAGE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "the page index is outside of the vmo");
        }

        locked_pages.store(page_idx as u64, page);
        Ok(())
    }

    /// Returns the status of writable mappings of the VMO.
    pub fn writable_mapping_status(&self) -> &WritableMappingStatus {
        // Only writable file-backed mappings may need to be tracked.
        debug_assert!(self.pager.is_some());
        &self.writable_mapping_status
    }
}

impl VmIo for Vmo {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        self.read(offset, writer)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        self.write(offset, reader)?;
        Ok(())
    }
}

impl VmIoFill for Vmo {
    fn fill_zeros(
        &self,
        offset: usize,
        len: usize,
    ) -> core::result::Result<(), (ostd::Error, usize)> {
        // TODO: Support efficient `fill_zeros()`.
        for i in 0..len {
            match self.write_slice(offset + i, &[0u8]) {
                Ok(()) => continue,
                Err(err) => return Err((err, i)),
            }
        }
        Ok(())
    }
}

/// Gets the page index range that contains the offset range of VMO.
pub fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}

/// The status of writable mappings of a VMO, i.e., shared mappings that may
/// include the `PROT_WRITE` permission.
///
/// Internally, it uses an `AtomicIsize` counter with the following rules:
///
/// - **Positive values**: number of active writable mappings.
/// - **Zero**: no writable mappings, and writable mappings are still allowed.
/// - **Negative values**: writable mappings are denied.
#[derive(Debug, Default)]
pub struct WritableMappingStatus(AtomicIsize);

impl WritableMappingStatus {
    /// Builds a new writable mapping.
    ///
    /// Fails with `EPERM` if writable mappings have been denied.
    pub(super) fn map(&self) -> Result<()> {
        // Increase unless negative
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v >= 0).then(|| v + 1)
            })
            .map_err(|_| Error::with_message(Errno::EPERM, "writable mappings have been denied"))?;
        Ok(())
    }

    /// Denies any future writable mapping.
    ///
    /// Fails with `EBUSY` if there are still active writable mappings.
    pub fn deny(&self) -> Result<()> {
        // Decrease unless positive
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v <= 0).then_some(-1)
            })
            .map_err(|_| {
                Error::with_message(Errno::EBUSY, "there are still active writable mappings")
            })?;
        Ok(())
    }

    /// Increments the writable mapping counter.
    ///
    /// Typically used when splitting an existing mapping, or forking a process.
    pub(super) fn increment(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the writable mapping counter.
    ///
    /// Typically used when unmapping a region, exiting a process, or merging mappings.
    pub(super) fn decrement(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
