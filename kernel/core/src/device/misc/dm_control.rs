// SPDX-License-Identifier: MPL-2.0

//! Linux Device Mapper control-device registration and userspace entry point.
//!
//! This module owns privilege checks, version negotiation, and ioctl ordering.
//! The command layer implements tableless identity management; the component
//! owns mapper indexes, device numbers, and admitted control references.

use aster_device_mapper::DmManager;
use device_id::{DeviceId, MinorId};
use ostd::mm::VmIo;
use spin::Once;

mod abi;
mod commands;
mod device_ops;
mod ioctl_params;
#[cfg(ktest)]
mod tests;

use abi::{DM_IOCTL_MAGIC, DM_VERSION_CMD, decode_command, validate_client_version, write_version};
use commands::handle_command;
use ioctl_params::process_ioctl_parameters;

use crate::{
    context::current_userspace,
    device::{Device, DeviceType, registry::char},
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::{
        UserNamespace,
        credentials::capabilities::CapSet,
        posix_thread::AsPosixThread,
        signal::{PollHandle, Pollable},
    },
    security::lsm::hooks as lsm_hooks,
    util::ioctl::RawIoctl,
};

const DM_CONTROL_MINOR: u32 = 236;

static DM_MANAGER: Once<DmManager> = Once::new();

/// Character device object backing `/dev/mapper/control`.
#[derive(Debug)]
struct DmControlDevice {
    /// Misc-device identity exposed through devtmpfs as `mapper/control`.
    id: DeviceId,
}

impl DmControlDevice {
    /// Creates the control device with the Linux-compatible misc minor number.
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
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

    /// Negotiates the DM ABI before copying or validating command parameters.
    ///
    /// Linux checks the caller's capability on every ioctl, including version
    /// queries. The version exchange touches only three `u32`s, even though the
    /// ioctl number encodes a full `dm_ioctl`. A version copyout fault takes
    /// precedence over an incompatible client version.
    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            current_thread!().as_posix_thread().unwrap(),
            CapSet::SYS_ADMIN,
        ))
        .is_err()
        {
            return_errno_with_message!(Errno::EACCES, "device-mapper ioctl requires CAP_SYS_ADMIN");
        }

        let raw_cmd = raw_ioctl.cmd();
        if (raw_cmd >> 8) & 0xff != DM_IOCTL_MAGIC {
            return_errno_with_message!(Errno::ENOTTY, "not a device-mapper ioctl");
        }

        // Keep version negotiation ahead of command lookup, as in Linux
        // drivers/md/dm-ioctl.c:check_version/ctl_ioctl (5078a3a23a1e).
        let mut version = [0u8; size_of::<[u32; 3]>()];
        current_userspace!().read_bytes(raw_ioctl.arg(), &mut version)?;
        let version_result = validate_client_version(&version);
        write_version(&mut version)?;
        current_userspace!().write_bytes(raw_ioctl.arg(), &version)?;
        version_result?;

        let command = decode_command(raw_cmd)?;
        if command == DM_VERSION_CMD {
            return Ok(0);
        }
        process_ioctl_parameters(
            command,
            &version,
            |offset, bytes| {
                let addr = raw_ioctl.arg().checked_add(offset).ok_or_else(|| {
                    Error::with_message(Errno::EFAULT, "ioctl user address overflow")
                })?;
                current_userspace!().read_bytes(addr, bytes)?;
                Ok(())
            },
            |len| {
                current_userspace!()
                    .writer(raw_ioctl.arg(), len)?
                    .fill_zeros(len)
                    .map_err(|(error, _)| error)?;
                Ok(())
            },
            |bytes| {
                current_userspace!().write_bytes(raw_ioctl.arg(), bytes)?;
                Ok(())
            },
            |buffer| handle_command(command, buffer),
        )?;
        Ok(0)
    }
}

/// Returns the manager initialized before exposing the control device.
fn manager() -> &'static DmManager {
    DM_MANAGER.get().unwrap()
}

/// Registers the control device after the misc major has been acquired.
pub(super) fn init_in_first_kthread() {
    DM_MANAGER.call_once(|| DmManager::new().unwrap());
    char::register(DmControlDevice::new()).unwrap();
}
