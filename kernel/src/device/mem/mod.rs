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

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
};

use aster_device::{register_device_ids, Device, DeviceId, DeviceIdAllocator, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, Result, SysAttrSetBuilder, SysBranchNode,
    SysPerms, SysStr,
};
use aster_util::printer::VmPrinter;
use file::MemFile;
pub use file::{getrandom, geturandom};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

use crate::{
    events::IoEvents,
    fs::{
        device::{add_device, DeviceFile},
        inode_handle::FileIo,
    },
    process::signal::{PollHandle, Pollable},
};

const MEM_MAJOR: u32 = 1;

/// A memory device.
#[derive(Debug)]
pub struct MemDevice {
    id: DeviceId,
    file: MemFile,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> Option<DeviceId> {
        Some(self.id)
    }

    fn sysnode(&self) -> Arc<dyn SysBranchNode> {
        self.weak_self().upgrade().unwrap()
    }
}

inherit_sys_branch_node!(MemDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        // Check if attribute exists
        if !self.fields.attr_set().contains(name) {
            return Err(Error::NotFound);
        }

        let attr = self.fields.attr_set().get(name).unwrap();
        // Check if attribute is readable
        if !attr.perms().can_read() {
            return Err(Error::PermissionDenied);
        }

        let mut printer = VmPrinter::new_skip(writer, offset);
        if name == "dev" {
            writeln!(printer, "{}:{}", self.id.major(), self.id.minor())
                .map_err(|_| Error::AttributeError)?
        };

        Ok(printer.bytes_written())
    }
});

#[inherit_methods(from = "self.fields")]
impl MemDevice {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> Result<()>;
    pub fn remove_child(&self, child_name: &str) -> Result<Arc<dyn SysBranchNode>>;
}

impl MemDevice {
    fn new(file: MemFile) -> Arc<Self> {
        let id = MEM_ID_ALLOCATOR
            .get()
            .unwrap()
            .allocate(file.minor())
            .unwrap();
        let name = SysStr::from(file.name().to_string());

        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| MemDevice {
            id,
            file,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl Drop for MemDevice {
    fn drop(&mut self) {
        MEM_ID_ALLOCATOR.get().unwrap().release(self.id.minor());
    }
}

impl FileIo for MemDevice {
    fn read(&self, writer: &mut VmWriter) -> crate::prelude::Result<usize> {
        self.file.read(writer)
    }

    fn write(&self, reader: &mut VmReader) -> crate::prelude::Result<usize> {
        self.file.write(reader)
    }
}

impl Pollable for MemDevice {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.file.poll(mask, poller)
    }
}

impl DeviceFile for MemDevice {
    fn open(&self) -> crate::prelude::Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(self.weak_self().upgrade().unwrap()))
    }
}

static MEM_ID_ALLOCATOR: Once<DeviceIdAllocator> = Once::new();

pub(super) fn init_in_first_process() {
    let ida = register_device_ids(DeviceType::Char, MEM_MAJOR, 0..256).unwrap();
    MEM_ID_ALLOCATOR.call_once(|| ida);

    add_device(MemDevice::new(MemFile::Full));
    add_device(MemDevice::new(MemFile::Null));
    add_device(MemDevice::new(MemFile::Random));
    add_device(MemDevice::new(MemFile::Urandom));
    add_device(MemDevice::new(MemFile::Zero));
}
