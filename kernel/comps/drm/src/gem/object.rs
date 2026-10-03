// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::BTreeMap, sync::Arc};
use core::fmt::Debug;

use aster_core::{fs::file::MappedObject, prelude::*};
use ostd::{mm::PAGE_SIZE, sync::Mutex};
use spin::Once;

use crate::gem::mmap_offset::DrmMmapOffset;

/// A graphics buffer object managed by the DRM GEM core.
#[derive(Debug)]
pub struct DrmGemObject {
    size: usize,
    mmap_offset: Once<DrmMmapOffset>,
    mmap_access_refs_by_client: Mutex<BTreeMap<u64, usize>>,
    backend: Arc<dyn DrmGemObjectBackend>,
}

impl DrmGemObject {
    /// Creates a GEM object with a page-aligned, nonzero size.
    pub fn new(size: usize, backend: Arc<dyn DrmGemObjectBackend>) -> Result<Self> {
        if size == 0 {
            return_errno_with_message!(Errno::EINVAL, "the GEM object size must not be zero");
        }
        if !size.is_multiple_of(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "the GEM object size is not page-aligned");
        }

        Ok(Self {
            size,
            mmap_offset: Once::new(),
            mmap_access_refs_by_client: Mutex::new(BTreeMap::new()),
            backend,
        })
    }

    /// Returns the page-aligned object size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Reads bytes from the object's backing storage.
    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let end = offset.checked_add(buf.len()).ok_or(Errno::EOVERFLOW)?;
        if end > self.size {
            return_errno_with_message!(Errno::EINVAL, "the GEM read exceeds the object");
        }

        self.backend.read_by_cpu(offset, buf)
    }

    /// Returns the userspace mmap offset, if allocated.
    pub(super) fn mmap_offset(&self) -> Option<DrmMmapOffset> {
        self.mmap_offset.get().copied()
    }

    /// Adds a client to the mmap access list.
    pub(crate) fn allow_mmap(&self, client_id: u64) {
        let mut access_refs = self.mmap_access_refs_by_client.lock();
        let count = access_refs.entry(client_id).or_insert(0);
        *count += 1;
    }

    /// Removes one client reference from the mmap access list.
    pub(crate) fn revoke_mmap(&self, client_id: u64) {
        let mut access_refs = self.mmap_access_refs_by_client.lock();
        let Some(count) = access_refs.get_mut(&client_id) else {
            return;
        };

        *count -= 1;
        if *count == 0 {
            access_refs.remove(&client_id);
        }
    }

    /// Returns whether the given client is currently allowed to map the object.
    pub(super) fn is_mmap_allowed(&self, client_id: u64) -> bool {
        self.mmap_access_refs_by_client
            .lock()
            .contains_key(&client_id)
    }

    pub(super) fn set_mmap_offset(&self, offset: DrmMmapOffset) {
        self.mmap_offset.call_once(move || offset);
    }

    /// Creates a VM-facing mapped object for an object-relative byte range.
    pub(super) fn create_mapped_object(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Box<dyn MappedObject>> {
        self.backend.create_mapped_object(offset, size)
    }
}

/// The device-provided memory backend of a GEM object.
pub trait DrmGemObjectBackend: Debug + Send + Sync {
    /// Reads object data into a kernel buffer using CPU access.
    ///
    /// Backends only need to implement this method when their object contents must
    /// be read directly by kernel code.
    ///
    /// Backends without CPU-readable storage may retain the default implementation.
    fn read_by_cpu(&self, _offset: usize, _buf: &mut [u8]) -> Result<()> {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "the GEM backend does not support CPU reads"
        );
    }

    /// Creates a VM-facing mapped object for an object-relative byte range.
    ///
    /// The returned mapped object must retain every backing resource that it needs
    /// after the GEM object and this backend are dropped.
    fn create_mapped_object(&self, offset: usize, size: usize) -> Result<Box<dyn MappedObject>>;
}
