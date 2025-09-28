// SPDX-License-Identifier: MPL-2.0

use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::SysBranchNode;

use super::inode_handle::FileIo;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Path,
        utils::{mkmod, InodeType},
    },
    prelude::*,
};

/// The abstract of device file.
pub trait DeviceFile: Device + FileIo {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>>;
}

struct DeviceFileWrapper {
    inner: Arc<dyn DeviceFile>,
}

impl DeviceFileWrapper {
    fn new(inner: Arc<dyn DeviceFile>) -> Arc<Self> {
        Arc::new(Self { inner })
    }
}

impl Debug for DeviceFileWrapper {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("DeviceFileWrapper").finish_non_exhaustive()
    }
}

impl Device for DeviceFileWrapper {
    fn device_type(&self) -> DeviceType {
        self.inner.device_type()
    }

    fn device_id(&self) -> Option<DeviceId> {
        self.inner.device_id()
    }

    fn sysnode(&self) -> Arc<dyn SysBranchNode> {
        self.inner.sysnode()
    }
}

/// Adds a device in `/sys/devices`.
pub fn add_device(device: Arc<dyn DeviceFile>) {
    let wrapper = DeviceFileWrapper::new(device);
    aster_device::add_device(wrapper);
}

/// Returns a specified device in `/sys/dev`.
pub fn get_device(type_: DeviceType, id: DeviceId) -> Option<Arc<dyn DeviceFile>> {
    aster_device::get_device(type_, id).map(|wrapper| {
        let wrapper = Arc::downcast::<DeviceFileWrapper>(wrapper).unwrap();
        wrapper.inner.clone()
    })
}

/// Returns all devices in `/sys/dev`.
pub fn all_devices() -> impl Iterator<Item = Arc<dyn DeviceFile>> {
    aster_device::all_devices().map(|wrapper| {
        let wrapper = Arc::downcast::<DeviceFileWrapper>(wrapper).unwrap();
        wrapper.inner.clone()
    })
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
