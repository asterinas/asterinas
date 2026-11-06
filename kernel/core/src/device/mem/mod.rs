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
//!
//! Each memory device is a device of the `mem` class in the device model,
//! which places it under `/sys/devices/virtual/mem/`,
//! indexes it from `/sys/class/mem/` and `/sys/dev/char/`,
//! and creates its devtmpfs node.
//! The same object is registered in the char registry so that opening the node reaches [`MemFile`].

mod file;

use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNode, DevNum},
};
use device_id::{DeviceId, MajorId, MinorId};
use file::MemFile;
pub(crate) use file::{getrandom, geturandom};
use spin::Once;

use super::{
    Device, DeviceType,
    registry::char::{self, MajorIdOwner},
};
use crate::{
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, mkmod},
    },
    prelude::*,
};

pub(super) fn init_in_first_kthread() {
    MEM_MAJOR.call_once(|| char::acquire_major(MajorId::new(1)).unwrap());
    MEM_CLASS.call_once(|| class::register(MemClass).unwrap());

    add_device(MemFile::Full).unwrap();
    add_device(MemFile::Null).unwrap();
    add_device(MemFile::Random).unwrap();
    add_device(MemFile::Urandom).unwrap();
    add_device(MemFile::Zero).unwrap();
}

/// The `mem` class: memory devices such as `/dev/null`.
pub(super) struct MemClass;

impl Class for MemClass {
    const NAME: &'static str = "mem";
    type Device = MemFile;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/mem.c#L685-L705>.
        let mode = match dev.payload() {
            MemFile::Full | MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                mkmod!(a+rw)
            }
            MemFile::Kmsg => mkmod!(a+r, u+w),
            _ => return None,
        };
        Some(DevNode {
            path: None,
            mode: Some(mode.bits()),
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
}

/// A memory device, as seen by the char registry.
pub(super) type MemDevice = ClassDevice<MemClass>;

impl Device for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.devnum()
            .expect("memory devices always have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // The device model creates the node when the device is added.
        None
    }

    fn open(&self, _context: &crate::device::DeviceOpenContext) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(*self.payload()))
    }
}

static MEM_MAJOR: Once<MajorIdOwner> = Once::new();
static MEM_CLASS: Once<Arc<ClassHandle<MemClass>>> = Once::new();

fn add_device(file: MemFile) -> Result<()> {
    let class = MEM_CLASS.get().unwrap();
    let id = DeviceId::new(MEM_MAJOR.get().unwrap().get(), MinorId::new(file.minor()));
    let device = MemDevice::builder(class, file.name(), file)
        .devnum(DevNum::char(id))
        .build();
    aster_device::add_device(&device)?;
    // The number-to-open map is the second registry; if it refuses the
    // device, the first registration is undone rather than left dangling.
    if let Err(error) = char::register(device.clone()) {
        let _ = aster_device::remove_device(&device);
        return Err(error);
    }
    Ok(())
}
