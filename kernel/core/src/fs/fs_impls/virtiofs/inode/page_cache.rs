// SPDX-License-Identifier: MPL-2.0

//! Page cache backend implementation for `VirtioFsInode`.

use core::ops::Deref;

use aster_fuse::{FuseCompletion, FuseNodeId, ReadReq, WriteFlags, WriteReq};
use aster_virtio::device::filesystem::{
    device::{FuseSession, MAX_READ_DATA_PAGES_PER_REQUEST, MAX_WRITE_DATA_PAGES_PER_REQUEST},
    pool::{FuseReplyBuf, FuseReplyBufs, FuseRequestBuf, FuseRequestBufs},
};
use io_util::batch::IoBatch;
use ostd::mm::{Segment, io::util::HasVmReaderWriter};
use smallvec::SmallVec;

use super::{super::open_handle::VirtioFsOpenHandle, VirtioFsInode};
use crate::{
    fs::file::AccessMode,
    prelude::*,
    vm::page_cache::{CachePage, CachePageExt, LockedCachePage, PageCacheBackend, PageRun},
};

impl PageCacheBackend for VirtioFsInode {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        validate_page_range(idx, 1, self.size())?;

        let handle = self.readable_page_handle()?;
        let nodeid = self.nodeid();
        let session = self.fs_ref().session().clone();
        let read_offset = idx * PAGE_SIZE;
        let mut chunk = ReadChunk::with_capacity(1);
        chunk.push(locked_page)?;
        // FIXME: Page-cache I/O should use the current `InodeHandle` status
        // flags instead of the flags captured in the cached FUSE handle. The
        // page-cache backend currently receives only the inode, so it cannot
        // observe per-open status flag changes.
        let read_req = ReadReq::new(
            handle.fh(),
            read_offset as u64,
            PAGE_SIZE as u32,
            handle.file_flags(),
        );

        submit_read_chunk(&session, nodeid, handle, read_req, chunk, io_batch)
    }

    fn read_pages_async(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let start_idx = pages.start_idx();
        validate_page_range(start_idx, pages.len(), self.size())?;

        let handle = self.readable_page_handle()?;
        let nodeid = self.nodeid();
        let session = self.fs_ref().session().clone();

        // Split the page run into device-limited request chunks, each issued
        // as its own FUSE_READ.
        while pages.len() > 0 {
            let chunk_len = pages.len().min(MAX_READ_DATA_PAGES_PER_REQUEST);
            let chunk_start = pages.start_idx();
            let read_offset = chunk_start * PAGE_SIZE;
            let read_len = chunk_len * PAGE_SIZE;
            let mut chunk = ReadChunk::with_capacity(chunk_len);

            for locked_page in (&mut pages).take(chunk_len) {
                chunk.push(locked_page)?;
            }

            let read_req = ReadReq::new(
                handle.fh(),
                read_offset as u64,
                read_len as u32,
                handle.file_flags(),
            );

            submit_read_chunk(&session, nodeid, handle.clone(), read_req, chunk, io_batch)?;
        }

        Ok(())
    }

    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let file_size = self.size();
        validate_page_range(idx, 1, file_size)?;

        let handle = self.writable_page_handle()?;
        let nodeid = self.nodeid();
        let session = self.fs_ref().session().clone();
        let write_offset = idx * PAGE_SIZE;
        let prepared = prepare_write_page(write_offset, locked_page, file_size, &session)?;
        let chunk = WriteChunk::single(prepared);

        // FIXME: Page-cache I/O should use the current `InodeHandle` status
        // flags instead of the flags captured in the cached FUSE handle. The
        // page-cache backend currently receives only the inode, so it cannot
        // observe per-open status flag changes.
        submit_write_chunk(&session, nodeid, handle, write_offset, chunk, io_batch)
    }

    fn write_pages_async(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let file_size = self.size();
        let start_idx = pages.start_idx();
        validate_page_range(start_idx, pages.len(), file_size)?;

        let handle = self.writable_page_handle()?;
        let nodeid = self.nodeid();
        let session = self.fs_ref().session().clone();

        // Bound each FUSE_WRITE payload by the negotiated `max_write`
        let max_pages_by_write = (session.max_write() as usize / PAGE_SIZE).max(1);

        while pages.len() > 0 {
            let chunk_len = pages
                .len()
                .min(MAX_WRITE_DATA_PAGES_PER_REQUEST)
                .min(max_pages_by_write);
            let chunk_start = pages.start_idx();
            let chunk_offset = chunk_start * PAGE_SIZE;

            // Keep pages locked until every page in the chunk is prepared. If
            // preparation fails, dropping these guards unlocks the pages
            // without leaving any of them in writeback state.
            let mut chunk = WriteChunk::with_capacity(chunk_len);
            for (chunk_pos, locked_page) in (&mut pages).enumerate().take(chunk_len) {
                let page_idx = chunk_start + chunk_pos;
                let page_start = page_idx * PAGE_SIZE;
                let prepared = prepare_write_page(page_start, locked_page, file_size, &session)?;
                chunk.push_prepared(prepared);
            }

            // On submission failure, roll the current chunk's pages back so the
            // caller can re-dirty them; pages already submitted in earlier
            // chunks complete through `io_batch` on their own.
            submit_write_chunk(
                &session,
                nodeid,
                handle.clone(),
                chunk_offset,
                chunk,
                io_batch,
            )?;
        }

        Ok(())
    }
}

struct ReadChunk {
    pages: SmallVec<[LockedCachePage; 1]>,
    data_bufs: FuseReplyBufs,
}

impl ReadChunk {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            pages: SmallVec::with_capacity(capacity),
            data_bufs: SmallVec::with_capacity(capacity),
        }
    }

    fn push(&mut self, locked_page: LockedCachePage) -> Result<()> {
        let cache_page = locked_page.deref().clone();
        let data_buf = FuseReplyBuf::new_map(Segment::from(cache_page).into())?;
        self.pages.push(locked_page);
        self.data_bufs.push(data_buf);
        Ok(())
    }
}

struct WriteChunk {
    prepared_pages: SmallVec<[PreparedWritePage; 1]>,
    data_bufs: FuseRequestBufs,
    total_len: usize,
}

impl WriteChunk {
    fn single(prepared: PreparedWritePage) -> Self {
        let mut chunk = Self::with_capacity(1);
        chunk.push_prepared(prepared);
        chunk
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            prepared_pages: SmallVec::with_capacity(capacity),
            data_bufs: SmallVec::with_capacity(capacity),
            total_len: 0,
        }
    }

    fn push_prepared(&mut self, prepared: PreparedWritePage) {
        self.total_len += prepared.writeback_len;
        self.prepared_pages.push(prepared);
    }

    fn start_writeback(self) -> (SmallVec<[CachePage; 1]>, FuseRequestBufs, usize) {
        let Self {
            prepared_pages,
            mut data_bufs,
            total_len,
        } = self;
        let mut pages = SmallVec::with_capacity(prepared_pages.len());
        for prepared in prepared_pages {
            let (page, data_buf) = prepared.start_writeback();
            pages.push(page);
            data_bufs.push(data_buf);
        }

        (pages, data_bufs, total_len)
    }
}

struct PreparedWritePage {
    writeback_len: usize,
    locked_page: LockedCachePage,
    data_buf: FuseRequestBuf,
}

impl PreparedWritePage {
    fn start_writeback(self) -> (CachePage, FuseRequestBuf) {
        self.locked_page.set_writing_back();

        (self.locked_page.unlock(), self.data_buf)
    }
}

impl VirtioFsInode {
    fn readable_page_handle(&self) -> Result<Arc<VirtioFsOpenHandle>> {
        if let Some(open_handle) = self.open_handles.find_readable_handle() {
            return Ok(open_handle);
        }

        self.open_transient_handle(AccessMode::O_RDONLY)
    }

    fn writable_page_handle(&self) -> Result<Arc<VirtioFsOpenHandle>> {
        if let Some(open_handle) = self.open_handles.find_writable_handle() {
            return Ok(open_handle);
        }

        self.open_transient_handle(AccessMode::O_RDWR)
    }

    pub(in crate::fs::fs_impls::virtiofs) fn invalidate_whole_page_cache(&self) -> Result<()> {
        self.inner.write().invalidate_page_cache()
    }
}

fn prepare_write_page(
    page_start: usize,
    locked_page: LockedCachePage,
    file_size: usize,
    session: &FuseSession,
) -> Result<PreparedWritePage> {
    // The completion callback clears the writeback flag without taking the
    // page lock, so it is safe to wait while retaining this guard.
    locked_page.wait_until_finish_writing_back();

    let writeback_len = PAGE_SIZE.min(file_size - page_start);
    let data_buf = session.alloc_write_buf(writeback_len)?;
    let mut page_reader = locked_page.reader();
    page_reader.limit(writeback_len);
    data_buf
        .writer()
        .unwrap()
        .write_fallible(&mut page_reader.to_fallible())?;

    Ok(PreparedWritePage {
        writeback_len,
        locked_page,
        data_buf,
    })
}

fn submit_read_chunk(
    session: &FuseSession,
    nodeid: FuseNodeId,
    handle: Arc<VirtioFsOpenHandle>,
    read_req: ReadReq,
    chunk: ReadChunk,
    io_batch: &mut IoBatch,
) -> Result<()> {
    let ReadChunk { pages, data_bufs } = chunk;
    let total_len = pages.len() * PAGE_SIZE;

    let complete_fn = move |status| {
        complete_read_pages(status, total_len, pages);
        // Keep the handle alive until the request completes or until the
        // completion closure is dropped on submission failure.
        drop(handle);
    };

    session
        .read_async(nodeid, read_req, data_bufs, Some(Box::new(complete_fn)))
        .map_err(Error::from)
        .map(|waiter| io_batch.push(waiter))
}

fn submit_write_chunk(
    session: &FuseSession,
    nodeid: FuseNodeId,
    handle: Arc<VirtioFsOpenHandle>,
    write_offset: usize,
    chunk: WriteChunk,
    io_batch: &mut IoBatch,
) -> Result<()> {
    let (pages, data_bufs, total_len) = chunk.start_writeback();

    let pages = Arc::new(pages);
    let completion_pages = pages.clone();
    let write_req = WriteReq::new(
        handle.fh(),
        write_offset as u64,
        total_len as u32,
        handle.file_flags(),
        WriteFlags::WRITE_CACHE,
    );

    let complete_fn = move |status| {
        // FIXME: Handle short FUSE writeback by invalidating or retrying unwritten
        // page contents. For now, continue with the usual writeback completion.
        if let FuseCompletion::MalformedResponse | FuseCompletion::RemoteError(_) = status {
            ostd::error!("virtiofs writeback failed; page data may be lost");
        }

        for page in completion_pages.iter() {
            page.clear_writing_back();
        }

        // Keep the handle alive until the request completes or until the
        // completion closure is dropped on submission failure.
        drop(handle);
    };

    match session.write_async(nodeid, write_req, data_bufs, Some(Box::new(complete_fn))) {
        Ok(waiter) => {
            io_batch.push(waiter);
            Ok(())
        }
        Err(err) => {
            for page in pages.iter().cloned() {
                let locked_page = page.lock();
                locked_page.set_dirty();
                locked_page.clear_writing_back();
            }
            Err(err.into())
        }
    }
}

fn complete_read_pages(
    status: FuseCompletion,
    read_size: usize,
    pages: impl IntoIterator<Item = LockedCachePage>,
) {
    let FuseCompletion::Complete(payload_len) = status else {
        return;
    };

    if payload_len > read_size {
        // The server returned more data than the request asked for; treat the
        // reply as malformed and leave the pages untouched.
        ostd::error!(
            "virtiofs read failed; payload length {} exceeds {}",
            payload_len,
            read_size
        );
        return;
    }

    for (page_idx, page) in pages.into_iter().enumerate() {
        let offset = page_idx * PAGE_SIZE;
        let received_len = payload_len.saturating_sub(offset).min(PAGE_SIZE);
        let padding_zero_len = PAGE_SIZE - received_len;
        if padding_zero_len != 0 {
            let mut writer = page.writer();
            writer.skip(received_len);
            writer.fill_zeros(padding_zero_len);
        }
        page.set_up_to_date();
    }
}

fn validate_page_range(start_idx: usize, page_count: usize, file_size: usize) -> Result<()> {
    if page_count == 0 {
        return Ok(());
    }

    let last_idx = start_idx
        .checked_add(page_count - 1)
        .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "virtiofs page index overflow"))?;
    let last_offset = last_idx
        .checked_mul(PAGE_SIZE)
        .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "virtiofs page offset overflow"))?;

    if last_offset >= file_size {
        return_errno_with_message!(Errno::EINVAL, "virtiofs page is beyond EOF");
    }

    Ok(())
}
