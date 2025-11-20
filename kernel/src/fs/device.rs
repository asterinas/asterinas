// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use super::inode_handle::FileIo;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Path,
        utils::{mkmod, InodeType},
    },
    prelude::*,
};

/// The abstraction of a device.
pub trait Device: Send + Sync + 'static {
    /// Returns the device type.
    fn type_(&self) -> DeviceType;

    /// Returns the device ID.
    fn id(&self) -> DeviceId;

    /// Opens a device.
    fn open(&self) -> Result<Box<dyn FileIo>>;
}

impl Debug for dyn Device {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("type", &self.type_())
            .field("id", &self.id())
            .finish()
    }
}

/// Device type
#[derive(Debug)]
pub enum DeviceType {
    Char,
    Block,
    Misc,
}

/// Adds a device node in `/dev`.
///
/// If the parent path does not exist, it will be created as a directory.
/// This function should be called when registering a device.
//
// TODO: Figure out what should happen when unregistering the device.
pub fn add_node(device: Arc<dyn Device>, path: &str, fs_resolver: &FsResolver) -> Result<Path> {
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
