// SPDX-License-Identifier: MPL-2.0

//! Memory devices.
//!
//! Character device with major number 1. The minor numbers are mapped as follows:
//! - 1 = /dev/mem      Physical memory access
//! - 2 = /dev/kmem     OBSOLETE - replaced by /proc/kcore
//! - 3 = /dev/null     Null device
//! - 4 = /dev/port     I/O port access
//! - 5 = /dev/zero     Null byte source
//! - 6 = /dev/core     OBSOLETE - replaced by /proc/kcore
//! - 7 = /dev/full     Returns ENOSPC on write
//! - 8 = /dev/random   Nondeterministic random number gen.
//! - 9 = /dev/urandom  Faster, less secure random number gen.
//! - 10 = /dev/aio     Asynchronous I/O notification interface
//! - 11 = /dev/kmsg    Writes to this come out as printk's, reads export the buffered printk records.
//! - 12 = /dev/oldmem  OBSOLETE - replaced by /proc/vmcore
//!
//! See <https://www.kernel.org/doc/Documentation/admin-guide/devices.txt>.

mod file;

use alloc::sync::Arc;

use device_id::{DeviceId, MajorId, MinorId};
use file::MemFile;
pub use file::{getrandom, geturandom};
use spin::Once;

use super::{
    Device, DeviceType, DevtmpfsInodeMeta,
    registry::char::{MajorIdOwner, acquire_major, register},
};
use crate::{
    fs::file::{FileIo, mkmod},
    prelude::*,
};

/// A memory device.
#[derive(Debug)]
pub struct MemDevice {
    id: DeviceId,
    file: MemFile,
}

impl MemDevice {
    fn new(file: MemFile) -> Self {
        let major = MEM_MAJOR.get().unwrap().get();
        let minor = MinorId::new(file.minor());

        Self {
            id: DeviceId::new(major, minor),
            file,
        }
    }
}

impl Device for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        // Linux's memory-device table uses nonzero modes only for devices
        // that override devtmpfs's default `u+rw` permissions.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/mem.c#L690>.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/mem.c#L734>.
        Some(match self.file {
            MemFile::Full | MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                DevtmpfsInodeMeta::with_mode(self.file.name(), mkmod!(a+rw))
            }
            MemFile::Kmsg => DevtmpfsInodeMeta::with_mode(self.file.name(), mkmod!(a+r, u+w)),
            _ => DevtmpfsInodeMeta::new(self.file.name()),
        })
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(self.file))
    }
}

static MEM_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    MEM_MAJOR.call_once(|| acquire_major(MajorId::new(1)).unwrap());

    register(Arc::new(MemDevice::new(MemFile::Full))).unwrap();
    register(Arc::new(MemDevice::new(MemFile::Null))).unwrap();
    register(Arc::new(MemDevice::new(MemFile::Random))).unwrap();
    register(Arc::new(MemDevice::new(MemFile::Urandom))).unwrap();
    register(Arc::new(MemDevice::new(MemFile::Zero))).unwrap();
}
