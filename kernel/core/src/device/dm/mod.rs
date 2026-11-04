// SPDX-License-Identifier: MPL-2.0

//! Linux Device Mapper control-device registration and device-model integration.
//!
//! The control module owns privilege checks and version negotiation.
//! Only `DM_VERSION` is supported.

use device_id::{DeviceId, MajorId, MinorId};

mod control;

use crate::{
    device::{Device, DeviceType, registry::char},
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

const DM_CONTROL_MINOR: u32 = 236;

/// Character device object backing `/dev/mapper/control`.
#[derive(Debug)]
struct DmControlDevice {
    /// Misc-device identity exposed through devtmpfs as `mapper/control`.
    id: DeviceId,
}

impl DmControlDevice {
    /// Creates the control device with the Linux-compatible misc minor number.
    fn new(major: MajorId) -> Arc<Self> {
        Arc::new(Self {
            id: DeviceId::new(major, MinorId::new(DM_CONTROL_MINOR)),
        })
    }
}

impl Device for DmControlDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("mapper/control").unwrap())
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(DmControlFile))
    }
}

/// Per-open file state for the stateless Device Mapper control node.
struct DmControlFile;

impl Pollable for DmControlFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // Global control events are not implemented; do not report spurious readiness.
        IoEvents::empty()
    }
}

impl FileOps for DmControlFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(
            Errno::EINVAL,
            "Device Mapper control device does not support read"
        )
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(
            Errno::EINVAL,
            "Device Mapper control device does not support write"
        )
    }
}

impl PerOpenFileOps for DmControlFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(
            Errno::ESPIPE,
            "Device Mapper control device does not support seek"
        )
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        control::ioctl(raw_ioctl)
    }
}

/// Registers the control device after the misc major has been acquired.
pub(super) fn init_in_first_kthread(major: MajorId) {
    char::register(DmControlDevice::new(major)).unwrap();
}
