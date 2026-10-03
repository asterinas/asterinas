// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc, vec::Vec};

use aster_core::{fs::file::MappedObject, prelude::*, vm::vmar::MapHandle};
use ostd::mm::{FrameAllocOptions, PAGE_SIZE, UFrame, VmIo};

use crate::gem::object::{DrmGemObject, DrmGemObjectBackend};

/// A VM-facing view into a shmem-backed GEM object.
#[derive(Debug)]
struct DrmGemShmemMappedObject {
    backend: DrmGemShmemBackend,
    object_offset: usize,
}

impl MappedObject for DrmGemShmemMappedObject {
    fn dup_at_offset(&self, offset: usize) -> Box<dyn MappedObject> {
        Box::new(Self {
            backend: self.backend.clone(),
            object_offset: self.object_offset + offset,
        })
    }

    fn handle_page_fault(&self, offset: usize, mut handle: MapHandle) -> Result<()> {
        let Some(object_offset) = self.object_offset.checked_add(offset) else {
            return_errno_with_message!(Errno::EFAULT, "the GEM page offset overflows");
        };
        let page_index = object_offset / PAGE_SIZE;
        let Some(page) = self.backend.pages.get(page_index) else {
            return_errno_with_message!(Errno::EFAULT, "the GEM page offset is out of bounds");
        };

        handle.map_frame(offset, page.clone());
        Ok(())
    }
}

/// A shared, RAM-backed GEM object backend.
///
/// Backing frames are currently allocated eagerly when an object is created.
/// This is intentional for the initial implementation.
/// Future work should allocate frames lazily on page faults
/// to avoid large upfront allocations.
#[derive(Clone, Debug)]
pub struct DrmGemShmemBackend {
    pages: Arc<[UFrame]>,
}

impl DrmGemShmemBackend {
    /// Creates a shmem-backed GEM object with a page-aligned size.
    pub fn new_object(size: usize) -> Result<Arc<DrmGemObject>> {
        let page_count = size / PAGE_SIZE;
        let mut pages = Vec::<UFrame>::with_capacity(page_count);

        for _ in 0..page_count {
            pages.push(FrameAllocOptions::new().alloc_frame()?.into());
        }
        let backend = Arc::new(Self {
            pages: pages.into(),
        });

        Ok(Arc::new(DrmGemObject::new(size, backend)?))
    }
}

impl DrmGemObjectBackend for DrmGemShmemBackend {
    fn read_by_cpu(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let mut object_offset = offset;
        let mut copied = 0;

        while copied < buf.len() {
            let page_index = object_offset / PAGE_SIZE;
            let page_offset = object_offset % PAGE_SIZE;
            let copy_len = (PAGE_SIZE - page_offset).min(buf.len() - copied);
            let page = self.pages.get(page_index).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "the GEM read exceeds the shmem object")
            })?;

            page.read_bytes(page_offset, &mut buf[copied..copied + copy_len])?;
            object_offset += copy_len;
            copied += copy_len;
        }

        Ok(())
    }

    fn create_mapped_object(&self, offset: usize, _size: usize) -> Result<Box<dyn MappedObject>> {
        Ok(Box::new(DrmGemShmemMappedObject {
            backend: self.clone(),
            object_offset: offset,
        }))
    }
}
