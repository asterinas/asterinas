// SPDX-License-Identifier: MPL-2.0

use super::inode_handle::FileIo;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Dentry,
        utils::{InodeMode, InodeType},
    },
    prelude::*,
};

/// The abstract of device
pub trait Device: FileIo {
    /// Return the device type.
    fn type_(&self) -> DeviceType;

    /// Return the device ID.
    fn id(&self) -> DeviceId;

    /// Open a device.
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(None)
    }
}

impl Debug for dyn Device {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("type", &self.type_())
            .field("id", &self.id())
            .finish()
    }
}

#[derive(Debug)]
/// Device type
pub enum DeviceType {
    CharDevice,
    BlockDevice,
    MiscDevice,
}

/// Device Id
#[derive(Clone, Copy)]
pub struct DeviceId(u64);

impl DeviceId {
    pub fn new(major: u32, minor: u32) -> Self {
        let major = major as u64;
        let minor = minor as u64;
        Self(
            (major & 0xffff_f000) << 32
                | (major & 0x0000_0fff) << 8
                | (minor & 0xffff_ff00) << 12
                | (minor & 0x0000_00ff),
        )
    }

    pub fn major(&self) -> u32 {
        ((self.0 >> 32) & 0xffff_f000 | (self.0 >> 8) & 0x0000_0fff) as u32
    }

    pub fn minor(&self) -> u32 {
        ((self.0 >> 12) & 0xffff_ff00 | self.0 & 0x0000_00ff) as u32
    }
}

impl Debug for DeviceId {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("DeviceId")
            .field("major", &self.major())
            .field("minor", &self.minor())
            .finish()
    }
}

impl From<DeviceId> for u64 {
    fn from(value: DeviceId) -> Self {
        value.0
    }
}

impl From<u64> for DeviceId {
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}

/// Add a device node to FS for the device.
///
/// If the parent path is not existing, `mkdir -p` the parent path.
/// This function is used in registering device.
pub fn add_node(device: Arc<dyn Device>, path: &str) -> Result<Dentry> {
    let mut dentry = {
        let fs_resolver = FsResolver::new();
        fs_resolver.lookup(&FsPath::try_from("/dev").unwrap())?
    };
    let mut relative_path = {
        let relative_path = path.trim_start_matches('/');
        if relative_path.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "invalid device path");
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

        match dentry.lookup(next_name) {
            Ok(next_dentry) => {
                if path_remain.is_empty() {
                    return_errno_with_message!(Errno::EEXIST, "device node is existing");
                }
                dentry = next_dentry;
            }
            Err(_) => {
                if path_remain.is_empty() {
                    // Create the device node
                    dentry = dentry.mknod(
                        next_name,
                        InodeMode::from_bits_truncate(0o666),
                        device.clone().into(),
                    )?;
                } else {
                    // Mkdir parent path
                    dentry = dentry.new_fs_child(
                        next_name,
                        InodeType::Dir,
                        InodeMode::from_bits_truncate(0o755),
                    )?;
                }
            }
        }
        relative_path = path_remain;
    }

    Ok(dentry)
}

/// Delete the device node from FS for the device.
///
/// This function is used in unregistering device.
pub fn delete_node(path: &str) -> Result<()> {
    let abs_path = {
        let device_path = path.trim_start_matches('/');
        if device_path.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "invalid device path");
        }
        String::from("/dev") + "/" + device_path
    };

    let (parent_dentry, name) = {
        let fs_resolver = FsResolver::new();
        fs_resolver.lookup_dir_and_base_name(&FsPath::try_from(abs_path.as_str()).unwrap())?
    };

    parent_dentry.unlink(&name)?;
    Ok(())
}
