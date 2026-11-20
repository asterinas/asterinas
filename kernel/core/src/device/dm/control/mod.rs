// SPDX-License-Identifier: MPL-2.0

//! Linux Device Mapper control device and file interface.
//!
//! The interface negotiates VERSION before command dispatch. Common ABI validation
//! and ioctl buffer handling are shared by commands added through sibling modules.
//! Device management, table operations, and target queries are not registered yet.

use device_id::{DeviceId, MinorId};
use ostd::mm::VmIo;

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

mod abi;
mod commands;
mod ioctl;

const DM_CONTROL_MINOR: u32 = 236;
const DM_IOCTL_MAGIC: u32 = 0xfd;
const DM_VERSION_CMD: u8 = 0;
const DM_VERSION: [u32; 3] = [4, 48, 0];

/// Character device backing `/dev/mapper/control` (misc major 10, minor 236).
#[derive(Debug)]
struct DmControlDevice {
    id: DeviceId,
}

impl DmControlDevice {
    fn new() -> Arc<Self> {
        let major = super::super::misc::MISC_MAJOR.get().unwrap().get();
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

/// An open handle for the Device Mapper control interface.
struct DmControlFile;

impl Pollable for DmControlFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
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

    /// Checks privilege and command type before exchanging the first three ABI words.
    ///
    /// As in Linux `ctl_ioctl`, copyout failure takes precedence over an
    /// incompatible client version, and command dispatch follows the handshake.
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

        let mut version = [0u8; size_of::<[u32; 3]>()];
        current_userspace!().read_bytes(raw_ioctl.arg(), &mut version)?;
        let version_result = validate_client_version(&version);
        write_version(&mut version);
        current_userspace!().write_bytes(raw_ioctl.arg(), &version)?;
        version_result?;

        let command = raw_cmd as u8;
        if command == DM_VERSION_CMD {
            return Ok(0);
        }
        // Linux negotiates the version before checking command support.
        let handler = commands::handler_for(command)?;
        ioctl::process_ioctl_parameters(raw_ioctl.arg(), &version, command, handler)
    }
}

fn validate_client_version(buffer: &[u8; 12]) -> Result<()> {
    if u32::from_le_bytes(buffer[..4].try_into().unwrap()) != DM_VERSION[0]
        || u32::from_le_bytes(buffer[4..8].try_into().unwrap()) > DM_VERSION[1]
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "incompatible device-mapper interface version"
        );
    }
    Ok(())
}

fn write_version(buffer: &mut [u8; 12]) {
    for (chunk, value) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(DM_VERSION) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn init_in_first_kthread() {
    char::register(DmControlDevice::new()).unwrap();
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn negotiates_backward_compatible_versions() {
        for requested in [[4, 0, 0], [4, 48, 0], [4, 48, u32::MAX]] {
            let mut buffer = [0u8; 12];
            for (chunk, value) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(requested) {
                chunk.copy_from_slice(&value.to_le_bytes());
            }
            validate_client_version(&buffer).unwrap();
            write_version(&mut buffer);
            assert_eq!(buffer, [4, 0, 0, 0, 48, 0, 0, 0, 0, 0, 0, 0]);
        }
    }

    #[ktest]
    fn rejects_incompatible_versions() {
        for requested in [[3u32, 0, 0], [5, 0, 0], [4, 49, 0]] {
            let mut buffer = [0u8; 12];
            for (chunk, value) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(requested) {
                chunk.copy_from_slice(&value.to_le_bytes());
            }
            assert_eq!(
                validate_client_version(&buffer).unwrap_err().error(),
                Errno::EINVAL
            );
            write_version(&mut buffer);
            assert_eq!(&buffer[..8], &[4, 0, 0, 0, 48, 0, 0, 0]);
        }
    }
}
