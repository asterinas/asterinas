// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_core::{ioctl::RawIoctl, prelude::*};
use ostd::sync::Mutex;
use sparse_id_alloc::SparseIdAlloc;

use crate::{DrmFile, DrmRect};

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

#[derive(Debug)]
struct DrmDeviceState {
    /// The currently active master context.
    ///
    /// Primary files retain their own `Arc<DrmMaster>`, so clearing this
    /// pointer on `DROP_MASTER` does not destroy the former master's context.
    master: Mutex<Option<Arc<DrmMaster>>>,
}

impl Default for DrmDeviceState {
    fn default() -> Self {
        Self {
            master: Mutex::new(None),
        }
    }
}

/// A master-owned context shared with associated primary files.
///
/// Exactly one DRM file owns this context, while other primary files may
/// retain references to it for legacy magic authentication. The context may
/// outlive its role as the device's current master.
#[derive(Debug)]
pub struct DrmMaster {
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

    pub(crate) fn allocate_magic(&self, target: &Arc<AtomicBool>) -> Result<u32> {
        let mut state = self.magic_state.lock();
        let Some(magic) = state.allocator.alloc() else {
            return_errno_with_message!(Errno::ENOMEM, "no DRM magic identifiers are available");
        };
        state.magic_table.insert(magic, Arc::downgrade(target));
        Ok(magic)
    }

    pub(crate) fn authenticate_magic(&self, magic: u32) -> Result<()> {
        let Some(target) = self
            .magic_state
            .lock()
            .magic_table
            .remove(&magic)
            .and_then(|target| target.upgrade())
        else {
            return_errno_with_message!(Errno::EINVAL, "the DRM magic identifier is invalid");
        };

        target.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn release_magic(&self, magic: u32) {
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
///
pub trait DrmDevice: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn features(&self) -> &DrmFeatures;
    fn caps(&self) -> &DrmDeviceCaps;

    /// Handles a driver-specific ioctl command.
    ///
    /// Returns `None` if the command is not recognized by the driver. The DRM core converts that
    /// case to `ENOTTY`; a recognized command returns its result through `Some`.
    fn handle_command(&self, _file: &DrmFile, _raw_ioctl: RawIoctl) -> Option<Result<i32>> {
        None
    }
}

impl dyn DrmDevice {
    pub(crate) fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.features().contains(feature)
    }
}

/// A registered DRM device together with its DRM-core-managed state.
#[derive(Debug)]
pub(crate) struct RegisteredDrmDevice {
    device: Arc<dyn DrmDevice>,
    state: DrmDeviceState,
}

impl RegisteredDrmDevice {
    pub(crate) fn new(device: Arc<dyn DrmDevice>) -> Self {
        Self {
            device,
            state: DrmDeviceState::default(),
        }
    }

    pub(crate) fn device(&self) -> &Arc<dyn DrmDevice> {
        &self.device
    }

    pub(crate) fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.device.has_feature(feature)
    }

    pub(crate) fn is_current_master(&self, client_id: u64) -> bool {
        let master = self.state.master.lock();
        master
            .as_ref()
            .is_some_and(|master| master.owner_client_id == client_id)
    }

    /// Authenticates a magic value on behalf of the current master.
    ///
    /// The current-master check and authentication are performed while holding
    /// the same device-state lock, so master ownership cannot change between
    /// authorization and the operation.
    pub(crate) fn authenticate_magic(&self, client_id: u64, magic: u32) -> Result<()> {
        let master = self.state.master.lock();
        let Some(current_master) = master
            .as_ref()
            .filter(|master| master.owner_client_id == client_id)
        else {
            return_errno_with_message!(Errno::EACCES, "the DRM client is not the current master");
        };

        current_master.authenticate_magic(magic)
    }

    /// Associates a newly opened primary file with a master context.
    ///
    /// Returns the associated context and whether the new file created that
    /// context and became the device's current master.
    pub(crate) fn open_primary_client(&self, client_id: u64) -> (Arc<DrmMaster>, bool) {
        let mut master = self.state.master.lock();
        match master.as_ref() {
            Some(master) => (master.clone(), false),
            None => {
                let new_master = Arc::new(DrmMaster::new(client_id));
                *master = Some(new_master.clone());
                (new_master, true)
            }
        }
    }

    /// Makes a primary client the device's current DRM master.
    ///
    /// A previous master reacquires its retained context. A file becoming master
    /// for the first time receives a new context.
    pub(crate) fn set_master(
        &self,
        client_id: u64,
        retained_master: Option<&Arc<DrmMaster>>,
    ) -> Result<Arc<DrmMaster>> {
        let mut master = self.state.master.lock();
        match master.as_ref() {
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
                let new_master = match retained_master {
                    Some(context) if context.owner_client_id == client_id => context.clone(),
                    Some(_) => return_errno_with_message!(
                        Errno::EINVAL,
                        "the retained DRM master belongs to another client"
                    ),
                    None => Arc::new(DrmMaster::new(client_id)),
                };
                *master = Some(new_master.clone());
                Ok(new_master)
            }
        }
    }

    /// Removes the device's current-master reference.
    ///
    /// The owning DRM file retains its own `Arc`, allowing it to reacquire the
    /// same context later.
    pub(crate) fn drop_master(&self, client_id: u64) -> Result<()> {
        let mut master = self.state.master.lock();
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
        const SHADOW_BUFFER         = 1 << 2;
        // Blows are an Asterinas-specific capability check used by this project and
        // is not treated as a direct Linux capability query in this abstraction.
        const DUMB_BUFFER           = 1 << 3;
        const PAGE_FLIP_TARGET      = 1 << 4;
    }
}

#[derive(Debug)]
pub struct DrmDeviceCaps {
    preferred_color_depth: u32,
    min_fb_rect: DrmRect,
    max_fb_rect: DrmRect,
    cursor_rect: DrmRect,

    flags: DrmDeviceCapFlags,
}

impl DrmDeviceCaps {
    /// Creates device capability values with validated geometry ranges.
    pub fn new(
        preferred_color_depth: u32,
        min_fb_rect: DrmRect,
        max_fb_rect: DrmRect,
        cursor_rect: DrmRect,
        flags: DrmDeviceCapFlags,
    ) -> Result<Self> {
        if !max_fb_rect.contains_rect(&min_fb_rect) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the minimum framebuffer rectangle is outside the maximum framebuffer rectangle"
            );
        }

        Ok(Self {
            preferred_color_depth,
            min_fb_rect,
            max_fb_rect,
            cursor_rect,
            flags,
        })
    }

    pub fn min_fb_rect(&self) -> DrmRect {
        self.min_fb_rect
    }

    pub fn max_fb_rect(&self) -> DrmRect {
        self.max_fb_rect
    }

    pub fn cursor_rect(&self) -> DrmRect {
        self.cursor_rect
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
            min_fb_rect: DrmRect::new(0, 0, 1, 1),
            max_fb_rect: DrmRect::new(0, 0, 4096, 4096),
            cursor_rect: DrmRect::new(0, 0, 64, 64),
            // TODO: Add FLIP_TARGET after finish page_flip with target.
            flags: DrmDeviceCapFlags::DUMB_BUFFER,
        }
    }
}
