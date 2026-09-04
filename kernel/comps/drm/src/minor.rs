// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    format,
    sync::{Arc, Weak},
};

use aster_core::{
    device::{Device, DeviceId, DeviceType, DevtmpfsNodeMeta, MajorId, MinorId},
    fs::PerOpenFileOps,
    prelude::*,
};

use crate::{device::RegisteredDrmDevice, file::DrmFile};

const DRM_MAJOR_ID: u16 = 226;
const RENDER_MINOR_BASE: u32 = 128;

#[derive(Debug, Clone, Copy)]
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
/// enforced at the file/ioctl level.
///
#[derive(Debug)]
pub(crate) struct DrmMinor {
    index: u32,
    type_: DrmMinorType,
    registered_device: Arc<RegisteredDrmDevice>,
    weak_self: Weak<Self>,
}

impl DrmMinor {
    pub(crate) fn new(
        index: u32,
        device: Arc<RegisteredDrmDevice>,
        type_: DrmMinorType,
    ) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Self {
            index,
            type_,
            registered_device: device,
            weak_self: weak_ref.clone(),
        })
    }

    pub(crate) fn type_(&self) -> DrmMinorType {
        self.type_
    }

    pub(crate) fn registered_device(&self) -> &Arc<RegisteredDrmDevice> {
        &self.registered_device
    }
}

impl Device for DrmMinor {
    fn id(&self) -> DeviceId {
        let minor_id = match self.type_ {
            DrmMinorType::Render => self.index + RENDER_MINOR_BASE,
            DrmMinorType::Primary => self.index,
            _ => unreachable!(),
        };
        DeviceId::new(MajorId::new(DRM_MAJOR_ID), MinorId::new(minor_id))
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        match self.type_ {
            DrmMinorType::Primary => {
                Some(DevtmpfsNodeMeta::new(format!("dri/card{}", self.index)).unwrap())
            }
            DrmMinorType::Render => Some(
                DevtmpfsNodeMeta::new(format!("dri/renderD{}", self.index + RENDER_MINOR_BASE))
                    .unwrap(),
            ),
            _ => None,
        }
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let Some(drm_minor) = self.weak_self.upgrade() else {
            return_errno_with_message!(Errno::EINVAL, "the DRM minor no longer exists");
        };
        Ok(Box::new(DrmFile::new(drm_minor)))
    }
}
