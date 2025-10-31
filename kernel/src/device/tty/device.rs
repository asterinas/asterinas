// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, SysAttrSetBuilder, SysBranchNode, SysPerms,
    SysStr,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;

use crate::{
    device::tty::{n_tty::ConsoleDriver, Tty, TTYAUX_ID_ALLOCATOR, TTY_ID_ALLOCATOR},
    events::IoEvents,
    fs::{device::DeviceFile, file_handle::Mappable, inode_handle::FileIo, utils::IoctlCmd},
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of current process.
#[derive(Debug)]
pub struct TtyDevice {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for TtyDevice {
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

inherit_sys_branch_node!(TtyDevice, fields, {
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
impl TtyDevice {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> aster_systree::Result<()>;
    pub fn remove_child(&self, child_name: &str) -> aster_systree::Result<Arc<dyn SysBranchNode>>;
}

impl TtyDevice {
    pub(super) fn new() -> Arc<Self> {
        let id = TTYAUX_ID_ALLOCATOR.get().unwrap().allocate(0).unwrap();
        let name = SysStr::from("tty");

        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| TtyDevice {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl Drop for TtyDevice {
    fn drop(&mut self) {
        TTYAUX_ID_ALLOCATOR.get().unwrap().release(0);
    }
}

impl FileIo for TtyDevice {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read tty device");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write tty device");
    }
}

impl Pollable for TtyDevice {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl DeviceFile for TtyDevice {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let Some(terminal) = current!().terminal() else {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the process does not have a controlling terminal"
            );
        };

        Ok(Some(terminal as Arc<dyn FileIo>))
    }
}

/// The `/dev/tty<N>` device.
#[derive(Debug)]
pub struct NttyDevice {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for NttyDevice {
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

inherit_sys_branch_node!(NttyDevice, fields, {
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
impl NttyDevice {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> aster_systree::Result<()>;
    pub fn remove_child(&self, child_name: &str) -> aster_systree::Result<Arc<dyn SysBranchNode>>;
}

impl NttyDevice {
    pub(super) fn new(index: u32) -> Arc<Self> {
        let id = TTY_ID_ALLOCATOR.get().unwrap().allocate(index).unwrap();
        let name = SysStr::from(format!("tty{}", index));

        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| NttyDevice {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl Drop for NttyDevice {
    fn drop(&mut self) {
        TTY_ID_ALLOCATOR.get().unwrap().release(self.id.minor());
    }
}

/// The `/dev/console` device.
#[derive(Debug)]
pub struct DevConsole {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
    tty: Weak<Tty<ConsoleDriver>>,
}

impl Device for DevConsole {
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
