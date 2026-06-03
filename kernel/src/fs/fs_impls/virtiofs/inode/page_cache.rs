// SPDX-License-Identifier: MPL-2.0

//! Page cache backend implementation for `VirtioFsInode`.

use core::ops::Deref;

use aster_fuse::{FuseCompletion, ReadReq, WriteFlags, WriteReq};
use aster_virtio::device::filesystem::pool::FuseReplyBuf;
use io_util::batch::IoBatch;
use ostd::mm::{Segment, io::util::HasVmReaderWriter};

use super::{super::open_handle::VirtioFsOpenHandle, VirtioFsInode};
use crate::{
    fs::file::AccessMode,
    prelude::*,
    vm::page_cache::{CachePageExt, LockedCachePage, PageCacheBackend},
};

impl PageCacheBackend for VirtioFsInode {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let handle = self.readable_page_handle()?;
        let nodeid = self.nodeid();
        let session = self.fs_ref().session().clone();
        let cache_page = locked_page.deref().clone();
        let data_buf = FuseReplyBuf::new_map(Segment::from(cache_page.clone()).into())?;
        let read_req = ReadReq::new(
            handle.fh(),
            page_offset(idx)? as u64,
            PAGE_SIZE as u32,
            handle.file_flags(),
        );

        let complete_fn = move |status| {
            let FuseCompletion::Complete(payload_len) = status else {
                return;
            };

            if payload_len > PAGE_SIZE {
                ostd::error!(
                    "virtiofs read failed for page index {}; payload length {} exceeds page size",
                    idx,
                    payload_len
                );
            } else if payload_len < PAGE_SIZE {
                let mut writer = cache_page.writer();
                writer.skip(payload_len);
                writer.fill_zeros(PAGE_SIZE - payload_len);
                locked_page.set_up_to_date();
            } else {
                locked_page.set_up_to_date();
            }
            // Keep the handle alive until the request completes
            // or until the completion closure is dropped on submission failure.
            drop(handle);
        };

        match session.read_async(nodeid, read_req, data_buf, Some(Box::new(complete_fn))) {
            Ok(waiter) => {
                io_batch.push(waiter);
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }

    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        locked_page.wait_until_finish_writing_back();

        let page_start = page_offset(idx)?;
        let file_size = self.size();
        if page_start >= file_size {
            return_errno_with_message!(Errno::EINVAL, "virtiofs writeback page is beyond EOF");
        }
        let writeback_len = PAGE_SIZE.min(file_size - page_start);

        let fs = self.fs_ref();
        let data_buf = fs.session().alloc_write_buf(writeback_len)?;
        let mut page_reader = locked_page.reader();
        page_reader.limit(writeback_len);

        data_buf
            .writer()
            .unwrap()
            .write_fallible(&mut page_reader.to_fallible())?;

        locked_page.set_writing_back();
        locked_page.set_up_to_date();

        let page = locked_page.unlock();

        let handle = match self.writable_page_handle() {
            Ok(handle) => handle,
            Err(err) => {
                let locked_page = page.lock();
                locked_page.set_dirty();
                locked_page.clear_writing_back();
                return Err(err);
            }
        };

        let nodeid = self.nodeid();
        let session = fs.session().clone();
        let complete_page = page.clone();
        let write_req = WriteReq::new(
            handle.fh(),
            page_start as u64,
            writeback_len as u32,
            handle.file_flags(),
            WriteFlags::empty(),
        );

        let complete_fn = move |status| {
            complete_page.clear_writing_back();
            if let FuseCompletion::MalformedResponse | FuseCompletion::RemoteError(_) = status {
                ostd::error!(
                    "virtiofs writeback failed for page index {}; data may be lost",
                    idx
                );
            }
            // Keep the handle alive until the request completes
            // or until the completion closure is dropped on submission failure.
            drop(handle);
        };

        match session.write_async(nodeid, write_req, data_buf, Some(Box::new(complete_fn))) {
            Ok(waiter) => {
                io_batch.push(waiter);
                Ok(())
            }
            Err(err) => {
                let locked_page = page.lock();
                locked_page.set_dirty();
                locked_page.clear_writing_back();
                Err(err.into())
            }
        }
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

fn page_offset(idx: usize) -> Result<usize> {
    idx.checked_mul(PAGE_SIZE)
        .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "virtiofs page offset overflow"))
}
