// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_core::prelude::*;
use ostd::sync::Mutex;
use sparse_id_alloc::SparseIdAlloc;

use crate::{DrmDeviceIndex, utils::DrmSize};

bitflags::bitflags! {
    pub struct DrmFeatures: u32 {
        const GEM              = 1 << 0;
        const MODESET          = 1 << 1;
        const RENDER           = 1 << 3;
        const ATOMIC           = 1 << 4;
        const SYNCOBJ          = 1 << 5;
        const SYNCOBJ_TIMELINE = 1 << 6;
        const COMPUTE_ACCEL    = 1 << 7;
        const GEM_GPUVA        = 1 << 8;
        const CURSOR_HOTSPOT   = 1 << 9;

        const USE_AGP          = 1 << 25;
        const LEGACY           = 1 << 26;
        const PCI_DMA          = 1 << 27;
        const SG               = 1 << 28;
        const HAVE_DMA         = 1 << 29;
        const HAVE_IRQ         = 1 << 30;
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
    fn device_caps(&self) -> &DrmDeviceCaps;
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
}

impl RegisteredDrmDevice {
    pub(super) fn new(device: Arc<dyn DrmDevice>) -> Result<Self> {
        Ok(Self {
            index: DrmDeviceIndex::alloc()?,
            device,
            master: Mutex::new(None),
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
}

bitflags::bitflags! {
    pub struct DrmDeviceCapFlags: u32 {
        const ASYNC_PAGE_FLIP       = 1 << 0;
        /// This field mainly exists for legacy compatibility and is the positive form of
        /// Linux `fb_modifiers_not_supported`.
        const FB_MODIFIERS          = 1 << 1;
        /// Indicates whether dumb-buffer should prefer shadow-buffer rendering.
        const PREFER_SHADOW         = 1 << 2;
    }
}

#[derive(Debug)]
pub struct DrmDeviceCaps {
    preferred_color_depth: u32,
    min_fb_size: DrmSize,
    max_fb_size: DrmSize,
    cursor_size: Option<DrmSize>,

    flags: DrmDeviceCapFlags,
}

impl DrmDeviceCaps {
    /// Creates device capability values with validated size limits.
    pub fn new(
        preferred_color_depth: u32,
        min_fb_size: DrmSize,
        max_fb_size: DrmSize,
        cursor_size: Option<DrmSize>,
        flags: DrmDeviceCapFlags,
    ) -> Result<Self> {
        if !min_fb_size.is_within(0..=max_fb_size.width(), 0..=max_fb_size.height()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the minimum framebuffer size exceeds the maximum framebuffer size"
            );
        }

        if cursor_size
            .is_some_and(|size| !size.is_within(1..=max_fb_size.width(), 1..=max_fb_size.height()))
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the cursor size is empty or exceeds the maximum framebuffer size"
            );
        }

        Ok(Self {
            preferred_color_depth,
            min_fb_size,
            max_fb_size,
            cursor_size,
            flags,
        })
    }

    pub fn min_fb_size(&self) -> DrmSize {
        self.min_fb_size
    }

    pub fn max_fb_size(&self) -> DrmSize {
        self.max_fb_size
    }

    pub fn cursor_size(&self) -> Option<DrmSize> {
        self.cursor_size
    }

    pub fn preferred_color_depth(&self) -> u32 {
        self.preferred_color_depth
    }

    pub fn flags(&self) -> DrmDeviceCapFlags {
        self.flags
    }
}

impl Default for DrmDeviceCaps {
    fn default() -> Self {
        Self {
            preferred_color_depth: 24,
            min_fb_size: DrmSize::new(1, 1),
            max_fb_size: DrmSize::new(4096, 4096),
            cursor_size: None,
            flags: DrmDeviceCapFlags::empty(),
        }
    }
}
