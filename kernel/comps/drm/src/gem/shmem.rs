// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc, vec::Vec};

use aster_core::{fs::file::MappedObject, prelude::*, vm::vmar::MapHandle};
use ostd::mm::{FrameAllocOptions, PAGE_SIZE, UFrame};

use crate::gem::object::{DrmGemObject, DrmGemObjectBackend};

/// A shared, RAM-backed GEM object backend.
///
/// Backing frames are currently allocated eagerly when an object is created.
/// This is intentional for the initial implementation,
/// future work should allocate frames lazily on page faults to avoid large upfront allocations.
#[derive(Debug)]
pub struct DrmGemShmemBackend {
    pages: Arc<[UFrame]>,
}

impl DrmGemShmemBackend {
    /// Creates a shmem-backed GEM object.
    pub fn new_object(size: usize) -> Result<Arc<DrmGemObject>> {
        if size == 0 {
            return_errno_with_message!(Errno::EINVAL, "the GEM object size must not be zero");
        }

        let Some(size) = size.checked_next_multiple_of(PAGE_SIZE) else {
            return_errno_with_message!(Errno::ENOMEM, "the page-aligned GEM object size overflows");
        };

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
    fn create_mapping(&self, offset: usize, _size: usize) -> Result<Box<dyn MappedObject>> {
        Ok(Box::new(DrmGemShmemMapping {
            pages: self.pages.clone(),
            object_offset: offset,
        }))
    }
}

/// One object-relative mapping of a shmem GEM backend.
#[derive(Debug)]
struct DrmGemShmemMapping {
    pages: Arc<[UFrame]>,
    object_offset: usize,
}

impl MappedObject for DrmGemShmemMapping {
    fn dup_at_offset(&self, offset: usize) -> Box<dyn MappedObject> {
        Box::new(Self {
            pages: self.pages.clone(),
            object_offset: self.object_offset + offset,
        })
    }

    fn handle_page_fault(&self, offset: usize, mut handle: MapHandle) -> Result<()> {
        let Some(object_offset) = self.object_offset.checked_add(offset) else {
            return_errno_with_message!(Errno::EFAULT, "the GEM page offset overflows");
        };
        let page_index = object_offset / PAGE_SIZE;
        let Some(page) = self.pages.get(page_index) else {
            return_errno_with_message!(Errno::EFAULT, "the GEM page offset is out of bounds");
        };

        handle.map_frame(offset, page.clone());
        Ok(())
    }
}
