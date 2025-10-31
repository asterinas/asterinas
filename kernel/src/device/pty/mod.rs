// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::TTYAUX_ID_ALLOCATOR,
    events::IoEvents,
    fs::{
        device::DeviceFile,
        devpts::DevPts,
        fs_resolver::{FsPath, FsResolver},
        inode_handle::FileIo,
        path::Path,
        utils::{mkmod, Inode, InodeType},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

mod driver;
mod master;

use aster_device::{register_device_ids, Device, DeviceId, DeviceIdAllocator, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, SysAttrSetBuilder, SysBranchNode, SysPerms,
    SysStr,
};
use aster_util::printer::VmPrinter;
pub use driver::PtySlave;
use inherit_methods_macro::inherit_methods;
pub use master::PtyMaster;
use spin::Once;

/// The ptmx device.
#[derive(Debug)]
pub struct PtmxDevice {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
    pts: Weak<DevPts>,
}

impl Device for PtmxDevice {
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

inherit_sys_branch_node!(PtmxDevice, fields, {
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
impl PtmxDevice {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> aster_systree::Result<()>;
    pub fn remove_child(&self, child_name: &str) -> aster_systree::Result<Arc<dyn SysBranchNode>>;
}

impl PtmxDevice {
    pub fn new(pts: Weak<DevPts>) -> Arc<Self> {
        let id = TTYAUX_ID_ALLOCATOR.get().unwrap().allocate(2).unwrap();
        let name = SysStr::from("ptmx");

        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| PtmxDevice {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
            pts,
        })
    }
}

impl Drop for PtmxDevice {
    fn drop(&mut self) {
        TTYAUX_ID_ALLOCATOR.get().unwrap().release(2);
    }
}

impl FileIo for PtmxDevice {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read ptmx");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write ptmx");
    }
}

impl Pollable for PtmxDevice {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl DeviceFile for PtmxDevice {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let devpts = self.pts.upgrade().unwrap();
        let (master, _) = devpts.create_master_slave_pair()?;
        Ok(Some(master as _))
    }
}

const UNIX98_PTY_SLAVE_MAJOR: u32 = 136;

pub static UNIX98_PTY_SLAVE_ID_ALLOCATOR: Once<DeviceIdAllocator> = Once::new();

static DEV_PTS: Once<Path> = Once::new();

pub fn init_in_first_process(fs_resolver: &FsResolver, ctx: &Context) -> Result<()> {
    UNIX98_PTY_SLAVE_ID_ALLOCATOR.call_once(|| {
        register_device_ids(DeviceType::Char, UNIX98_PTY_SLAVE_MAJOR, 0..256).unwrap()
    });

    let dev = fs_resolver.lookup(&FsPath::try_from("/dev")?)?;
    // Create the "pts" directory and mount devpts on it.
    let devpts_path = dev.new_fs_child("pts", InodeType::Dir, mkmod!(a+rx, u+w))?;
    let devpts_mount = devpts_path.mount(DevPts::new(), ctx)?;

    DEV_PTS.call_once(|| Path::new_fs_root(devpts_mount));

    // Create the "ptmx" symlink.
    let ptmx = dev.new_fs_child("ptmx", InodeType::SymLink, mkmod!(a+rwx))?;
    ptmx.inode().write_link("pts/ptmx")?;
    Ok(())
}

pub fn new_pty_pair(index: u32, ptmx: Arc<dyn Inode>) -> Result<(Arc<PtyMaster>, Arc<PtySlave>)> {
    debug!("pty index = {}", index);
    let master = PtyMaster::new(ptmx, index);
    let slave = master.slave().clone();
    Ok((master, slave))
}
