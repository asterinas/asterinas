// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, SysAttrSetBuilder, SysBranchNode, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;

use super::inode_handle::FileIo;
use crate::{
    events::IoEvents,
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Path,
        utils::{mkmod, InodeType},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// The abstract of device file.
pub trait DeviceFile: Device + FileIo {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>>;
}

struct AllDevices {
    block_devices: BTreeMap<u64, Arc<dyn DeviceFile>>,
    char_devices: BTreeMap<u64, Arc<dyn DeviceFile>>,
}

static ALL_DEVICES: Mutex<AllDevices> = Mutex::new(AllDevices {
    block_devices: BTreeMap::new(),
    char_devices: BTreeMap::new(),
});

/// Adds a device in `/sys/devices`.
pub fn add_device(device: Arc<dyn DeviceFile>) {
    let mut all_devices = ALL_DEVICES.lock();
    match device.type_() {
        DeviceType::Block => {
            all_devices
                .block_devices
                .insert(device.id().unwrap().as_encoded_u64(), device.clone());
        }
        DeviceType::Char => {
            all_devices
                .char_devices
                .insert(device.id().unwrap().as_encoded_u64(), device.clone());
        }
        _ => panic!("unsupported device type"),
    }

    aster_device::add_device(device);
}

/// Returns a specified device if it exists.
pub fn get_device(type_: DeviceType, id: DeviceId) -> Option<Arc<dyn DeviceFile>> {
    let all_devices = ALL_DEVICES.lock();
    match type_ {
        DeviceType::Block => all_devices.block_devices.get(&id.as_encoded_u64()).cloned(),
        DeviceType::Char => all_devices.char_devices.get(&id.as_encoded_u64()).cloned(),
        _ => panic!("unsupported device type"),
    }
}

/// Returns all registered devices in `/sys/dev`.
pub fn all_devices() -> impl Iterator<Item = Arc<dyn DeviceFile>> {
    aster_device::all_devices().filter_map(|dev| get_device(dev.type_(), dev.id().unwrap()))
}

/// Adds a device node in `/dev`.
///
/// If the parent path does not exist, it will be created as a directory.
/// This function should be called when registering a device.
//
// TODO: Figure out what should happen when unregistering the device.
pub fn add_node(device: Arc<dyn DeviceFile>, path: &str, fs_resolver: &FsResolver) -> Result<Path> {
    let mut dev_path = fs_resolver.lookup(&FsPath::try_from("/dev").unwrap())?;
    let mut relative_path = {
        let relative_path = path.trim_start_matches('/');
        if relative_path.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the device path is invalid");
        }
        relative_path
    };

    while !relative_path.is_empty() {
        let (next_name, path_remain) = if let Some((prefix, suffix)) = relative_path.split_once('/')
        {
            (prefix, suffix.trim_start_matches('/'))
        } else {
            (relative_path, "")
        };

        match dev_path.lookup(next_name) {
            Ok(next_path) => {
                if path_remain.is_empty() {
                    return_errno_with_message!(Errno::EEXIST, "the device node already exists");
                }
                dev_path = next_path;
            }
            Err(_) => {
                if path_remain.is_empty() {
                    // Create the device node
                    dev_path = dev_path.mknod(next_name, mkmod!(a+rw), device.clone().into())?;
                } else {
                    // Create the parent directory
                    dev_path =
                        dev_path.new_fs_child(next_name, InodeType::Dir, mkmod!(a+rx, u+w))?;
                }
            }
        }
        relative_path = path_remain;
    }

    Ok(dev_path)
}

#[derive(Debug)]
pub struct DummyDevice {
    type_: DeviceType,
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for DummyDevice {
    fn type_(&self) -> DeviceType {
        self.type_
    }

    fn id(&self) -> Option<DeviceId> {
        Some(self.id)
    }

    fn sysnode(&self) -> Arc<dyn SysBranchNode> {
        self.weak_self().upgrade().unwrap()
    }
}

inherit_sys_branch_node!(DummyDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[inherit_methods(from = "self.fields")]
impl DummyDevice {
    pub fn weak_self(&self) -> &Weak<Self>;
}

impl DummyDevice {
    pub fn new(type_: DeviceType, id: DeviceId) -> Arc<Self> {
        let name = SysStr::from(format!("dummy{}", id.as_encoded_u64()));
        let attrs = SysAttrSetBuilder::new()
            .build()
            .expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| DummyDevice {
            type_,
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl FileIo for DummyDevice {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::ENXIO, "cannot read dummy device");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::ENXIO, "cannot write dummy device");
    }
}

impl Pollable for DummyDevice {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl DeviceFile for DummyDevice {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        return_errno_with_message!(Errno::ENXIO, "cannot open dummy device");
    }
}
