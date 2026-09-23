// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_core::{fs::file::MappedObject, prelude::*};
use ostd::{mm::PAGE_SIZE, sync::Mutex};
use sparse_id_alloc::SparseIdAlloc;

use crate::gem::{DrmGemOps, mmap_offset::DrmGemMmapOffsetSpace, object::DrmGemObject};

static DRM_DEVICE_INDEX_ALLOCATOR: Mutex<SparseIdAlloc> = Mutex::new(SparseIdAlloc::new(0, 63));

/// Defines the top-level contract of a DRM device instance.
///
/// `DrmDevice` is the composition root for device-facing DRM behavior.
/// It provides stable identity metadata and shared capability discovery,
/// while higher-level DRM operations are expected to be layered as
/// dedicated operation traits.
pub trait DrmDevice: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn features(&self) -> &DrmFeatures;
    fn has_features(&self, feature: DrmFeatures) -> bool {
        self.features().contains(feature)
    }

    /// Returns the device's GEM operations when the GEM feature is enabled.
    fn as_gem_ops(&self) -> Option<&dyn DrmGemOps>;
}

bitflags::bitflags! {
    /// Capabilities provided by an Asterinas DRM device implementation.
    ///
    /// These flags are internal to the DRM subsystem. Their bit positions
    /// are not part of the DRM userspace ABI.
    pub struct DrmFeatures: u32 {
        /// Supports creation of a render device node.
        const RENDER           = 1 << 0;
        /// Supports DRM synchronization objects.
        const SYNCOBJ          = 1 << 1;
        /// Supports timeline synchronization objects.
        const SYNCOBJ_TIMELINE = 1 << 2;
        /// Requires userspace-aware cursor hotspot handling.
        const CURSOR_HOTSPOT   = 1 << 3;
    }
}

/// A registered DRM device together with its DRM-core-managed state.
#[derive(Debug)]
pub(super) struct RegisteredDrmDevice {
    index: DrmDeviceIndex,
    device: Arc<dyn DrmDevice>,
    /// The currently active master context.
    ///
    /// Primary files retain their own `Arc<DrmMaster>`, so clearing this
    /// pointer on `DROP_MASTER` does not destroy the former master's context.
    master: Mutex<Option<Arc<DrmMaster>>>,
    /// The device-wide fake mmap-offset space for GEM objects, if GEM is supported.
    mmap_offset_space: Option<Mutex<DrmGemMmapOffsetSpace>>,
}

impl RegisteredDrmDevice {
    pub(super) fn new(device: Arc<dyn DrmDevice>) -> Result<Self> {
        let mmap_offset_space = device
            .as_gem_ops()
            .map(|_| Mutex::new(DrmGemMmapOffsetSpace::default()));

        Ok(Self {
            index: DrmDeviceIndex::alloc()?,
            device,
            master: Mutex::new(None),
            mmap_offset_space,
        })
    }

    pub(super) fn index(&self) -> u32 {
        self.index.index()
    }

    pub(super) fn device(&self) -> &Arc<dyn DrmDevice> {
        &self.device
    }

    pub(super) fn is_client_master(&self, client_id: u64) -> bool {
        let master = self.master.lock();
        master
            .as_ref()
            .is_some_and(|master| master.owner_client_id == client_id)
    }

    /// Authenticates a magic value on behalf of the current master.
    ///
    /// The current-master check and authentication are performed while holding
    /// the same master lock, so master ownership cannot change between
    /// authorization and the operation.
    pub(super) fn authenticate_magic(&self, client_id: u64, magic: u32) -> Result<()> {
        let master = self.master.lock();
        let Some(current_master) = master
            .as_ref()
            .filter(|master| master.owner_client_id == client_id)
        else {
            return_errno_with_message!(Errno::EACCES, "the DRM client is not the current master");
        };

        current_master.authenticate_magic(magic)
    }

    /// Associates a newly opened primary file with a master context.
    pub(super) fn open_primary_client(&self, client_id: u64) -> Arc<DrmMaster> {
        let mut master = self.master.lock();
        match master.as_ref() {
            Some(master) => master.clone(),
            None => {
                let new_master = Arc::new(DrmMaster::new(client_id));
                *master = Some(new_master.clone());
                new_master
            }
        }
    }

    /// Makes a primary client the device's current DRM master.
    ///
    /// A previous master reacquires its retained context. A file becoming master
    /// for the first time receives a new context.
    pub(super) fn set_master(
        &self,
        client_id: u64,
        retained_master: Option<&Arc<DrmMaster>>,
    ) -> Result<Arc<DrmMaster>> {
        let mut current_master = self.master.lock();
        match current_master.as_ref() {
            Some(current_master) => {
                if current_master.owner_client_id == client_id {
                    Ok(current_master.clone())
                } else {
                    return_errno_with_message!(
                        Errno::EBUSY,
                        "another DRM client is already the current master"
                    )
                }
            }
            None => {
                let master = match retained_master {
                    Some(retained) if retained.owner_client_id == client_id => retained.clone(),
                    Some(_) => return_errno_with_message!(
                        Errno::EINVAL,
                        "the retained DRM master belongs to another client"
                    ),
                    None => Arc::new(DrmMaster::new(client_id)),
                };
                *current_master = Some(master.clone());
                Ok(master)
            }
        }
    }

    /// Removes the device's current-master reference.
    ///
    /// The owning DRM file retains its own `Arc`, allowing it to reacquire the
    /// same context later.
    pub(super) fn drop_master(&self, client_id: u64) -> Result<()> {
        let mut master = self.master.lock();
        if !master
            .as_ref()
            .is_some_and(|master| master.owner_client_id == client_id)
        {
            return_errno_with_message!(Errno::EINVAL, "the DRM client is not the current master");
        }
        *master = None;

        Ok(())
    }

    pub(super) fn ensure_gem_has_allocated_range(&self, object: &Arc<DrmGemObject>) -> Result<()> {
        self.mmap_offset_space
            .as_ref()
            .ok_or_else(|| {
                Error::with_message(Errno::EOPNOTSUPP, "the DRM device does not support GEM")
            })?
            .lock()
            .ensure_allocated(object)?;
        Ok(())
    }

    pub(super) fn create_gem_mapping(
        &self,
        client_id: u64,
        offset: usize,
        size: usize,
    ) -> Result<Box<dyn MappedObject>> {
        // All supported targets have pointer widths no greater than 64 bits,
        // so converting page counts from `usize` to `u64` is lossless.
        let start_page = (offset / PAGE_SIZE) as u64;
        let page_count = (size / PAGE_SIZE) as u64;
        let (gem_object, object_page_offset) = {
            let mmap_offset_space = self
                .mmap_offset_space
                .as_ref()
                .ok_or_else(|| {
                    Error::with_message(Errno::EOPNOTSUPP, "the DRM device does not support GEM")
                })?
                .lock();

            mmap_offset_space
                .lookup(start_page, page_count)
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::EINVAL,
                        "the mmap range does not belong to a GEM object",
                    )
                })?
        };

        if !gem_object.is_mmap_allowed(client_id) {
            return_errno_with_message!(Errno::EACCES, "the GEM object is not accessible");
        }

        let object_offset = object_page_offset.checked_mul(PAGE_SIZE).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the GEM mapping offset overflows")
        })?;
        gem_object.create_mapping(object_offset, size)
    }
}

/// An index shared by all minor nodes belonging to a DRM device.
#[derive(Debug)]
struct DrmDeviceIndex(u32);

impl DrmDeviceIndex {
    fn alloc() -> Result<Self> {
        let Some(index) = DRM_DEVICE_INDEX_ALLOCATOR.lock().alloc() else {
            return_errno_with_message!(Errno::ENOMEM, "no DRM device indices are available");
        };

        Ok(Self(index))
    }

    fn index(&self) -> u32 {
        self.0
    }
}

impl Drop for DrmDeviceIndex {
    fn drop(&mut self) {
        DRM_DEVICE_INDEX_ALLOCATOR.lock().free(self.0);
    }
}

/// A master-owned context shared with associated primary files.
///
/// Exactly one DRM file owns this context, while other primary files may
/// retain references to it for legacy magic authentication.
/// The context may outlive its role as the device's current master.
#[derive(Debug)]
pub(super) struct DrmMaster {
    owner_client_id: u64,
    magic_state: Mutex<DrmMagicState>,
}

impl DrmMaster {
    fn new(owner_client_id: u64) -> Self {
        Self {
            owner_client_id,
            magic_state: Mutex::new(DrmMagicState {
                allocator: SparseIdAlloc::new(1, u32::MAX),
                magic_table: BTreeMap::new(),
            }),
        }
    }

    /// Returns the client ID of the file that owns this master context.
    pub(super) fn owner_client_id(&self) -> u64 {
        self.owner_client_id
    }

    pub(super) fn allocate_magic(&self, authenticated: &Arc<AtomicBool>) -> Result<u32> {
        let mut state = self.magic_state.lock();
        let Some(magic) = state.allocator.alloc() else {
            return_errno_with_message!(Errno::ENOMEM, "no DRM magic identifiers are available");
        };
        state
            .magic_table
            .insert(magic, Arc::downgrade(authenticated));
        Ok(magic)
    }

    pub(super) fn authenticate_magic(&self, magic: u32) -> Result<()> {
        let Some(authenticated) = self
            .magic_state
            .lock()
            .magic_table
            .remove(&magic)
            .and_then(|authenticated| authenticated.upgrade())
        else {
            return_errno_with_message!(Errno::EINVAL, "the DRM magic identifier is invalid");
        };

        authenticated.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub(super) fn release_magic(&self, magic: u32) {
        let mut state = self.magic_state.lock();
        state.magic_table.remove(&magic);
        state.allocator.free(magic);
    }
}

/// Magic IDs and their pending authentication targets.
///
/// Both fields are protected by the same lock so an ID cannot be reused while
/// its authentication entry is still pending. Authentication consumes the
/// table entry, but the ID remains allocated until the DRM file is released.
#[derive(Debug)]
struct DrmMagicState {
    allocator: SparseIdAlloc,
    magic_table: BTreeMap<u32, Weak<AtomicBool>>,
}
