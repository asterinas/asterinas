// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(unused_variables)]

//! Virtual Memory Objects (VMOs).

use core::{
    cell::RefCell,
    ops::Range,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use ostd::{
    mm::{VmIo, VmIoFill, VmReader, VmWriter, io_util::HasVmReaderWriter},
    task::disable_preempt,
};
use xarray::{Cursor, LockedXArray, XArray};

use crate::{
    fs::utils::{CachePage, CachePageExt, LockedCachePage, PageCacheBackend, PageState},
    prelude::*,
};

mod options;
mod page_cache;
mod pager;

pub use options::VmoOptions;

/// A trait for providing pages to a VMO.
trait Pager: Send + Sync {
    /// Commits a page at the given index.
    ///
    /// If the page is already committed, it returns the existing page.
    /// If not, it allocates or fetches a new page as needed.
    ///
    /// The `will_overwrite` flag indicates whether the page will be completely
    /// overwritten after being committed. If it is true, the initialization from
    /// the backend can be skipped.
    fn commit_on(
        &self,
        locked_pages: LockedXArray<'_, CachePage>,
        page_idx: usize,
        will_overwrite: bool,
    ) -> Result<CachePage>;

    /// Attempts to commit a page using a cursor, without blocking on I/O.
    ///
    /// If the page is already present and initialized, returns it immediately.
    /// Otherwise, returns an error indicating what action is needed (I/O or waiting).
    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
    ) -> core::result::Result<Option<(usize, CachePage)>, VmoCommitError>;

    /// Flushes dirty pages in the specified range to the backend storage.
    ///
    /// This is a no-op for anonymous pagers.
    fn flush_range(
        &self,
        locked_pages: LockedXArray<'_, CachePage>,
        page_idx_range: Range<usize>,
    ) -> Result<()> {
        Ok(())
    }

    /// Returns the backend storage, if any.
    fn backend(&self) -> Option<&Arc<dyn PageCacheBackend>> {
        None
    }
}

/// Pager for anonymous (RAM-only) VMOs.
struct AnonymousPager;
/// Pager for file-backed VMOs.
struct FileBackedPager {
    /// The backend that provides file I/O operations.
    backend: Arc<dyn PageCacheBackend>,
}

impl Pager for AnonymousPager {
    fn commit_on(
        &self,
        mut locked_pages: LockedXArray<'_, CachePage>,
        page_idx: usize,
        _will_overwrite: bool,
    ) -> Result<CachePage> {
        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            return Ok(page.clone());
        }

        let new_page = CachePage::alloc_zero()?;
        cursor.store(new_page.clone());

        Ok(new_page)
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
    ) -> core::result::Result<Option<(usize, CachePage)>, VmoCommitError> {
        if let Some(committed_page) = cursor.load() {
            return Ok(Some((cursor.index() as usize, committed_page.clone())));
        }

        Ok(None)
    }
}

impl Pager for FileBackedPager {
    fn commit_on(
        &self,
        mut locked_pages: LockedXArray<'_, CachePage>,
        page_idx: usize,
        will_overwrite: bool,
    ) -> Result<CachePage> {
        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            let page = page.clone();
            if self.backend.npages() > page_idx {
                drop(locked_pages);

                page.wait_until_init();
            }

            return Ok(page);
        };

        if self.backend.npages() > page_idx {
            let new_page = CachePage::alloc_uninit()?;
            // Acquiring the lock from a new page must succeed.
            let locked_page = new_page.try_lock().unwrap();

            cursor.store(locked_page.clone());

            drop(locked_pages);

            if will_overwrite {
                Ok(locked_page.unlock())
            } else {
                self.backend.read_page(page_idx, locked_page)?;
                Ok(new_page)
            }
        } else {
            let new_page = CachePage::alloc_zero()?;
            cursor.store(new_page.clone());

            Ok(new_page)
        }
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
    ) -> core::result::Result<Option<(usize, CachePage)>, VmoCommitError> {
        let page_idx = cursor.index() as usize;

        let Some(page) = cursor.load() else {
            return Err(VmoCommitError::NeedIo(page_idx));
        };

        if self.backend.npages() > page_idx && page.load_state() == PageState::Uninit {
            return Err(VmoCommitError::WaitUntilInit(page_idx, page.clone()));
        }

        Ok(Some((page_idx, page.clone())))
    }

    fn backend(&self) -> Option<&Arc<dyn PageCacheBackend>> {
        Some(&self.backend)
    }

    fn flush_range(
        &self,
        locked_pages: LockedXArray<'_, CachePage>,
        page_idx_range: Range<usize>,
    ) -> Result<()> {
        let mut flushable_pages = Vec::new();
        let npages = self.backend.npages();
        if page_idx_range.start >= npages {
            return Ok(());
        }

        let mut cursor = locked_pages.cursor(page_idx_range.start as u64);
        if let Some(page) = cursor.load()
            && page.is_dirty()
        {
            flushable_pages.push((page_idx_range.start, page.clone()));
        }

        while let Some(page_idx) = cursor.next_present() {
            let page_idx = page_idx as usize;
            if page_idx >= page_idx_range.end || page_idx >= npages {
                break;
            }

            let page = cursor.load().unwrap();
            flushable_pages.push((page_idx, page.clone()));
        }

        drop(locked_pages);

        for (page_idx, page) in flushable_pages {
            let locked_page = page.lock();
            if locked_page.is_dirty() {
                locked_page.set_up_to_date();
                self.backend.write_page(page_idx, locked_page)?;
            }
        }

        Ok(())
    }
}

/// Virtual Memory Objects (VMOs) represent contiguous ranges of virtual memory pages.
///
/// VMOs serve as the fundamental building blocks for memory management in Asterinas,
/// providing a unified interface for both anonymous (RAM-backed) and file-backed memory.
///
/// # Types of VMOs
///
/// There are two primary types of VMOs, distinguished by their backing storage:
///
/// 1. **Anonymous VMO**: Backed purely by RAM with no persistent storage. Pages are
///    initially zero-filled and exist only in memory. These are typically used for
///    heap allocations, anonymous mappings, and stack memory.
///
/// 2. **File-backed VMO**: Associated with a file through a [`PageCacheBackend`].
///    Pages are lazily loaded from the file on first access and can be written back
///    to storage. These VMOs integrate with the page cache for efficient file I/O
///    and memory-mapped files.
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
/// around its low-level counterpart [`ostd::mm::CachePage`].
/// Compared with `CachePage`,
/// `Vmo` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
pub struct Vmo {
    /// The backend that provides pages for this VMO.
    pager: Box<dyn Pager>,
    /// Flags
    flags: VmoFlags,
    /// The virtual pages where the VMO resides.
    pages: XArray<CachePage>,
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
    /// A general error occurred during the commit operation.
    Err(Error),
    /// The commit operation requires an I/O operation to read the page
    /// from the backend.
    ///
    /// The wrapped value is the page index.
    NeedIo(usize),
    /// Failed to lock the page because it's currently locked by another thread.
    ///
    /// Contains the page index and the page itself.
    LockPageFailed(usize, CachePage),
    /// The page exists but is not yet initialized.
    ///
    /// The caller should wait for initialization to complete.
    /// Contains the page index and the page.
    WaitUntilInit(usize, CachePage),
    /// The page is currently being written back to storage.
    ///
    /// The caller should wait for the writeback to complete.
    /// Contains the page index and the locked page.
    WaitUntilFinishWriteback(usize, LockedCachePage),
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

impl Vmo {
    /// Commits a page at a specific page index.
    ///
    /// This method may involve I/O operations if the VMO needs to fetch a page from
    /// the underlying page cache.
    pub fn commit_on(&self, page_idx: usize) -> Result<CachePage> {
        self.commit_on_internal(page_idx, false)
    }

    fn commit_on_internal(&self, page_idx: usize, need_init: bool) -> Result<CachePage> {
        let locked_pages = self.pages.lock();
        if page_idx * PAGE_SIZE > self.size() {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        self.pager.commit_on(locked_pages, page_idx, need_init)
    }

    /// Commits the page corresponding to the target offset in the VMO.
    ///
    /// If the commit operation needs to perform I/O, it will return a [`VmoCommitError::NeedIo`].
    pub fn try_commit_page(
        &self,
        offset: usize,
    ) -> core::result::Result<CachePage, VmoCommitError> {
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
            .map(|(_, page)| page)
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
    ) -> core::result::Result<(usize, CachePage), VmoCommitError> {
        if let Some((index, frame)) = self.pager.try_commit_with_cursor(cursor)? {
            Ok((index, frame))
        } else {
            let index = cursor.index() as usize;
            Ok((index, self.commit_on_internal(index, false)?))
        }
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
            &mut dyn FnMut() -> core::result::Result<(usize, CachePage), VmoCommitError>,
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

    /// Returns the size of current VMO.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Returns the flags of current VMO.
    pub fn flags(&self) -> VmoFlags {
        self.flags
    }

    /// Returns the status of writable mappings of the VMO.
    pub fn writable_mapping_status(&self) -> &WritableMappingStatus {
        // Only writable file-backed mappings may need to be tracked.
        debug_assert!(self.pager.backend().is_some());
        &self.writable_mapping_status
    }

    fn decommit_pages(
        &self,
        mut locked_pages: LockedXArray<CachePage>,
        range: Range<usize>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        let mut cursor = locked_pages.cursor_mut(page_idx_range.start as u64);

        loop {
            cursor.remove();
            let page_idx = cursor.next_present();
            if page_idx.is_none_or(|idx| idx >= page_idx_range.end as u64) {
                break;
            }
        }

        Ok(())
    }
}

impl Vmo {
    /// Reads the specified amount of buffer content starting from the target offset in the VMO.
    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        let mut read_range = offset..(offset + read_len);
        let mut read_offset = offset % PAGE_SIZE;

        let mut read = move |commit_fn: &mut dyn FnMut() -> core::result::Result<
            (usize, CachePage),
            VmoCommitError,
        >| {
            let (_, page) = commit_fn()?;
            page.reader()
                .skip(read_offset)
                .read_fallible(writer)
                .map_err(|e| VmoCommitError::from(e.0))?;
            read_offset = 0;
            Ok(())
        };

        'retry: loop {
            let res = self.try_operate_on_range(&read_range, &mut read);
            match res {
                Ok(_) => return Ok(()),
                Err(VmoCommitError::Err(e)) => return Err(e),
                Err(VmoCommitError::NeedIo(index)) => {
                    self.commit_on(index)?;
                    read_range.start = index * PAGE_SIZE;
                    continue 'retry;
                }
                Err(VmoCommitError::WaitUntilInit(index, cache_page)) => {
                    cache_page.wait_until_init();
                    read_range.start = index * PAGE_SIZE;
                    continue 'retry;
                }
                _ => unreachable!(),
            }
        }
    }

    /// Writes the specified amount of buffer content starting from the target offset in the VMO.
    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain();
        let write_range = offset..(offset + write_len);
        let mut write_offset = offset % PAGE_SIZE;

        if self.pager.backend().is_none() {
            let write = move |commit_fn: &mut dyn FnMut() -> core::result::Result<
                (usize, CachePage),
                VmoCommitError,
            >| {
                let (_, page) = commit_fn()?;
                page.writer()
                    .skip(write_offset)
                    .write_fallible(reader)
                    .map_err(|e| VmoCommitError::from(e.0))?;
                write_offset = 0;
                Ok(())
            };

            self.write_on_range(
                write_range.clone(),
                write,
                Option::<fn(&LockedCachePage) -> Result<()>>::None,
                false,
            )
        } else {
            let reader = RefCell::new(reader);
            let write_offset = RefCell::new(write_offset);
            let mut write = |commit_fn: &mut dyn FnMut() -> core::result::Result<
                (usize, CachePage),
                VmoCommitError,
            >| {
                let (index, page) = commit_fn()?;
                let locked_page = page
                    .try_lock()
                    .ok_or_else(|| VmoCommitError::LockPageFailed(index, page))?;
                if locked_page.is_writing_back() {
                    return Err(VmoCommitError::WaitUntilFinishWriteback(index, locked_page));
                }
                locked_page.set_dirty();
                locked_page
                    .writer()
                    .skip(*write_offset.borrow())
                    .write_fallible(&mut reader.borrow_mut())
                    .map_err(|e| VmoCommitError::from(e.0))?;
                *write_offset.borrow_mut() = 0;
                Ok(())
            };

            let mut fallback_write = |locked_page: &LockedCachePage| {
                locked_page
                    .writer()
                    .skip(*write_offset.borrow())
                    .write_fallible(&mut reader.borrow_mut())?;
                *write_offset.borrow_mut() = 0;
                Ok(())
            };

            if write_range.len() < PAGE_SIZE {
                self.write_on_range(write_range.clone(), write, Some(fallback_write), false)?;
            } else {
                let temp = write_range.start + PAGE_SIZE - 1;
                let up_align_start = temp - temp % PAGE_SIZE;
                let down_align_end = write_range.end - write_range.end % PAGE_SIZE;
                if write_range.start != up_align_start {
                    let head_range = write_range.start..up_align_start;
                    self.write_on_range(head_range, &mut write, Some(&mut fallback_write), false)?;
                }
                if up_align_start != down_align_end {
                    let mid_range = up_align_start..down_align_end;
                    self.write_on_range(mid_range, &mut write, Some(&mut fallback_write), true)?;
                }
                if down_align_end != write_range.end {
                    let tail_range = down_align_end..write_range.end;
                    self.write_on_range(tail_range, &mut write, Some(&mut fallback_write), false)?;
                }
            }

            Ok(())
        }
    }

    fn write_on_range<F1, F2>(
        &self,
        mut range: Range<usize>,
        mut operate: F1,
        mut fallback: Option<F2>,
        need_init: bool,
    ) -> Result<()>
    where
        F1: FnMut(
            &mut dyn FnMut() -> core::result::Result<(usize, CachePage), VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
        F2: FnMut(&LockedCachePage) -> Result<()>,
    {
        'retry: loop {
            let res = self.try_operate_on_range(&range, &mut operate);
            match res {
                Ok(_) => return Ok(()),
                Err(VmoCommitError::Err(e)) => return Err(e),
                Err(VmoCommitError::NeedIo(index)) => {
                    self.commit_on_internal(index, need_init)?;
                    range.start = index * PAGE_SIZE;
                    continue 'retry;
                }
                Err(VmoCommitError::WaitUntilInit(index, cache_page)) => {
                    cache_page.wait_until_init();
                    range.start = index * PAGE_SIZE;
                    continue 'retry;
                }
                Err(VmoCommitError::LockPageFailed(index, cache_page)) => {
                    let Some(fallback) = &mut fallback else {
                        unreachable!()
                    };
                    let locked_page = cache_page.lock();
                    locked_page.wait_until_finish_write_back();
                    locked_page.set_dirty();

                    fallback(&locked_page)?;
                    range.start = (index + 1) * PAGE_SIZE;
                    continue 'retry;
                }
                Err(VmoCommitError::WaitUntilFinishWriteback(index, locked_page)) => {
                    let Some(fallback) = &mut fallback else {
                        unreachable!()
                    };
                    locked_page.wait_until_finish_write_back();
                    locked_page.set_dirty();

                    fallback(&locked_page)?;
                    range.start = (index + 1) * PAGE_SIZE;
                    continue 'retry;
                }
            }
        }
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
