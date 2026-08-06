// SPDX-License-Identifier: MPL-2.0

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use aster_drm::DrmDevice;
use device_id::{DeviceId, MajorId, MinorId};

use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta, drm::file::DrmFile},
    fs::file::PerOpenFileOps,
    prelude::*,
};

const DRM_MAJOR_ID: u16 = 226;
const RENDER_MINOR_BASE: u32 = 128;

#[derive(Debug, Clone, Copy)]
pub(super) enum DrmMinorType {
    Primary = 0,
    #[expect(dead_code)]
    Control = 1,
    Render = 2,
    #[expect(dead_code)]
    Accel = 32,
}

/// Represents a DRM minor node exposed to userspace (e.g. primary, render,
/// or control node).
///
/// A `DrmMinor` corresponds to a single character device registered under
/// `/dev/dri/` (such as `/dev/dri/cardX` or `/dev/dri/renderDX`). It does not
/// own hardware state by itself; instead, it provides a userspace-facing
/// access point with a specific permission and usage model.
///
/// Multiple `DrmMinor` instances may reference the same underlying
/// `DrmDevice`, sharing the same driver instance and global device state.
/// The semantic differences between minors (e.g. authentication requirements,
/// ioctl visibility, access restrictions) are expressed via `type_` and
/// enforced at the file/ioctl level.
///
#[derive(Debug)]
pub(super) struct DrmMinor {
    index: u32,
    type_: DrmMinorType,

    device: Arc<dyn DrmDevice>,

    next_file_id: AtomicU32,
    master: Mutex<Option<u32>>,

    weak_self: Weak<Self>,
}

impl DrmMinor {
    pub(super) fn new(index: u32, device: Arc<dyn DrmDevice>, type_: DrmMinorType) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Self {
            index,
            type_,
            device,
            next_file_id: AtomicU32::new(0),
            master: Mutex::new(None),
            weak_self: weak_ref.clone(),
        })
    }

    pub(super) fn type_(&self) -> DrmMinorType {
        self.type_
    }

    pub(super) fn alloc_file_id(&self) -> u32 {
        self.next_file_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn is_master(&self, file_id: u32) -> bool {
        let master = self.master.lock();
        *master == Some(file_id)
    }

    pub(super) fn set_master(&self, file_id: u32) -> Result<()> {
        let mut master = self.master.lock();
        if *master == Some(file_id) {
            return Ok(());
        }
        if master.is_some() {
            return_errno!(Errno::EBUSY);
        }
        *master = Some(file_id);
        Ok(())
    }

    pub(super) fn drop_master(&self, file_id: u32) {
        let mut master = self.master.lock();
        if *master == Some(file_id) {
            *master = None;
        }
    }

    pub(super) fn device(&self) -> &Arc<dyn DrmDevice> {
        &self.device
    }
}

impl Device for DrmMinor {
    fn id(&self) -> DeviceId {
        let minor_id = match self.type_ {
            DrmMinorType::Render => self.index + RENDER_MINOR_BASE,
            _ => self.index,
        };
        DeviceId::new(MajorId::new(DRM_MAJOR_ID), MinorId::new(minor_id))
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        match self.type_ {
            DrmMinorType::Primary => {
                Some(DevtmpfsInodeMeta::new(format!("dri/card{}", self.index)))
            }
            DrmMinorType::Render => Some(DevtmpfsInodeMeta::new(format!(
                "dri/renderD{}",
                self.index + RENDER_MINOR_BASE
            ))),
            _ => None,
        }
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let drm_minor = self.weak_self.upgrade().ok_or(Errno::EINVAL)?;

        let file_id = self.alloc_file_id();

        // Linux-compatible bootstrap behavior: on the primary node (`/dev/dri/cardX`),
        // the first opened file becomes the DRM master automatically.
        if matches!(self.type_, DrmMinorType::Primary) {
            let mut master = self.master.lock();
            if master.is_none() {
                *master = Some(file_id);
            }
        }

        Ok(Box::new(DrmFile::new(file_id, drm_minor)))
    }
}
