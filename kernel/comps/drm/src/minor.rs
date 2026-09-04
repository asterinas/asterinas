// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    format,
    sync::{Arc, Weak},
};

use aster_core::{
    device::{Device, DeviceType},
    fs::{devtmpfs::DevtmpfsNodeMeta, file::PerOpenFileOps},
    prelude::*,
};
use device_id::{DeviceId, MajorId, MinorId};

use crate::{
    device::{DrmDevice, DrmMaster, RegisteredDrmDevice},
    file::DrmFile,
};

const DRM_MAJOR_ID: u16 = 226;
const RENDER_MINOR_BASE: u32 = 128;

#[derive(Clone, Copy, Debug)]
pub enum DrmMinorType {
    Primary = 0,
    Control = 1,
    Render = 2,
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
/// enforced by the minor and ioctl layers.
#[derive(Debug)]
pub(super) struct DrmMinor {
    type_: DrmMinorType,
    registered_device: Arc<RegisteredDrmDevice>,
    weak_self: Weak<Self>,
}

impl DrmMinor {
    pub(super) fn new(
        registered_device: Arc<RegisteredDrmDevice>,
        type_: DrmMinorType,
    ) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Self {
            type_,
            registered_device,
            weak_self: weak_ref.clone(),
        })
    }

    pub(super) fn type_(&self) -> DrmMinorType {
        self.type_
    }

    pub(super) fn device(&self) -> &Arc<dyn DrmDevice> {
        self.registered_device.device()
    }

    /// Opens a client through this minor and returns its master context, if applicable.
    pub(super) fn open_client(&self, client_id: u64) -> Option<Arc<DrmMaster>> {
        match self.type_ {
            DrmMinorType::Primary => Some(self.registered_device.open_primary_client(client_id)),
            _ => None,
        }
    }

    pub(super) fn is_client_master(&self, client_id: u64) -> bool {
        match self.type_ {
            DrmMinorType::Primary => self.registered_device.is_client_master(client_id),
            _ => false,
        }
    }

    pub(super) fn authenticate_magic(&self, client_id: u64, magic: u32) -> Result<()> {
        self.check_primary()?;
        self.registered_device.authenticate_magic(client_id, magic)
    }

    pub(super) fn set_master(
        &self,
        client_id: u64,
        retained_master: Option<&Arc<DrmMaster>>,
    ) -> Result<Arc<DrmMaster>> {
        self.check_primary()?;
        self.registered_device
            .set_master(client_id, retained_master)
    }

    pub(super) fn drop_master(&self, client_id: u64) -> Result<()> {
        self.check_primary()?;
        self.registered_device.drop_master(client_id)
    }

    fn check_primary(&self) -> Result<()> {
        if !matches!(self.type_, DrmMinorType::Primary) {
            return_errno_with_message!(Errno::EACCES, "the DRM operation requires a primary node");
        }

        Ok(())
    }
}

impl Device for DrmMinor {
    fn id(&self) -> DeviceId {
        let index = self.registered_device.index();
        let minor_id = match self.type_ {
            DrmMinorType::Render => index + RENDER_MINOR_BASE,
            DrmMinorType::Primary => index,
            _ => unreachable!(),
        };
        DeviceId::new(MajorId::new(DRM_MAJOR_ID), MinorId::new(minor_id))
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        let index = self.registered_device.index();
        match self.type_ {
            DrmMinorType::Primary => {
                Some(DevtmpfsNodeMeta::new(format!("dri/card{}", index)).unwrap())
            }
            DrmMinorType::Render => Some(
                DevtmpfsNodeMeta::new(format!("dri/renderD{}", index + RENDER_MINOR_BASE)).unwrap(),
            ),
            _ => None,
        }
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let drm_minor = self.weak_self.upgrade().unwrap();
        Ok(Box::new(DrmFile::new(drm_minor)))
    }
}
