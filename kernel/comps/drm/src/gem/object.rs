// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::BTreeMap, sync::Arc};
use core::fmt::Debug;

use aster_core::{fs::file::MappedObject, prelude::*};
use ostd::{mm::PAGE_SIZE, sync::Mutex};
use spin::Once;

/// A graphics buffer object managed by the DRM GEM core.
#[derive(Debug)]
pub struct DrmGemObject {
    size: usize,
    mmap_offset_start_page: Once<u64>,
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
            mmap_offset_start_page: Once::new(),
            mmap_access_refs_by_client: Mutex::new(BTreeMap::new()),
            backend,
        })
    }

    /// Returns the page-aligned object size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns the userspace mmap offset in bytes, or 0 if unallocated.
    pub(crate) fn mmap_offset_bytes(&self) -> u64 {
        let start_page = self.mmap_offset_start_page.get().copied().unwrap_or(0);

        start_page * PAGE_SIZE as u64
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
    pub(crate) fn is_mmap_allowed(&self, client_id: u64) -> bool {
        self.mmap_access_refs_by_client
            .lock()
            .contains_key(&client_id)
    }

    pub(super) fn has_mmap_offset(&self) -> bool {
        self.mmap_offset_start_page.is_completed()
    }

    pub(super) fn set_mmap_offset_start_page(&self, start_page: u64) {
        self.mmap_offset_start_page.call_once(move || start_page);
    }

    /// Resolves an object-relative range into a mapping understood by the VM core.
    pub(crate) fn create_mapping(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Box<dyn MappedObject>> {
        self.backend.create_mapping(offset, size)
    }
}

/// The device-provided memory backend of a GEM object.
pub trait DrmGemObjectBackend: Debug + Send + Sync {
    /// Resolves a validated object-relative byte range into a VM mapping.
    ///
    /// The returned mapping must retain every backing resource that it needs
    /// after the GEM object and this backend are dropped.
    fn create_mapping(&self, offset: usize, size: usize) -> Result<Box<dyn MappedObject>>;
}
