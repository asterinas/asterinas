// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, SysAttrSetBuilder, SysBranchNode, SysPerms,
    SysStr,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use super::Tty;
use crate::{
    device::{tty::n_tty::ConsoleDriver, TTYAUX_ID_ALLOCATOR},
    events::IoEvents,
    fs::{device::DeviceFile, file_handle::Mappable, inode_handle::FileIo, utils::IoctlCmd},
    prelude::Result,
    process::signal::{PollHandle, Pollable},
};

/// The `/dev/console` device.
#[derive(Debug)]
pub struct DevConsole {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
    tty: Weak<Tty<ConsoleDriver>>,
}

impl Device for DevConsole {
    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn device_id(&self) -> Option<DeviceId> {
        Some(self.id)
    }
}

inherit_sys_branch_node!(DevConsole, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
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
impl DevConsole {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> aster_systree::Result<()>;
    pub fn remove_child(&self, child_name: &str) -> aster_systree::Result<Arc<dyn SysBranchNode>>;
}

impl DevConsole {
    pub(super) fn new(tty: &Arc<Tty<ConsoleDriver>>) -> Arc<Self> {
        let id = TTYAUX_ID_ALLOCATOR.get().unwrap().allocate(1).unwrap();
        let name = SysStr::from("console");

        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| DevConsole {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
            tty: Arc::downgrade(tty),
        })
    }

    fn as_tty(&self) -> Arc<Tty<ConsoleDriver>> {
        self.tty.upgrade().unwrap()
    }
}

impl Drop for DevConsole {
    fn drop(&mut self) {
        TTYAUX_ID_ALLOCATOR.get().unwrap().release(self.id.minor());
    }
}

impl Pollable for DevConsole {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.as_tty().poll(mask, poller)
    }
}

impl FileIo for DevConsole {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.as_tty().read(writer)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.as_tty().write(reader)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.as_tty().ioctl(cmd, arg)
    }

    fn mappable(&self) -> Result<Mappable> {
        self.as_tty().mappable()
    }
}

impl DeviceFile for DevConsole {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(self.as_tty()))
    }
}
