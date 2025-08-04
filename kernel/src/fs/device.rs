// SPDX-License-Identifier: MPL-2.0

use super::inode_handle::FileIo;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Path,
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

/// A device ID, containing a major device number and a minor device number.
#[derive(Clone, Copy, Debug)]
pub struct DeviceId {
    major: u32,
    minor: u32,
}

impl DeviceId {
    /// Creates a device ID from the major device number and the minor device number.
    pub fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Returns the major device number.
    pub fn major(&self) -> u32 {
        self.major
    }

    /// Returns the minor device number.
    pub fn minor(&self) -> u32 {
        self.minor
    }
}

impl DeviceId {
    /// Creates a device ID from the encoded `u64` value.
    ///
    /// See [`as_encoded_u64`] for details about how to encode a device ID to a `u64` value.
    ///
    /// [`as_encoded_u64`]: Self::as_encoded_u64
    pub fn from_encoded_u64(raw: u64) -> Self {
        let major = ((raw >> 32) & 0xffff_f000 | (raw >> 8) & 0x0000_0fff) as u32;
        let minor = ((raw >> 12) & 0xffff_ff00 | raw & 0x0000_00ff) as u32;
        Self::new(major, minor)
    }

    /// Encodes the device ID as a `u64` value.
    ///
    /// The lower 32 bits use the same encoding strategy as Linux. See the Linux implementation at:
    /// <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/include/linux/kdev_t.h#L39-L44>.
    ///
    /// If the major or minor device number is too large, the additional bits will be recorded
    /// using the higher 32 bits. Note that as of 2025, the Linux kernel still has no support for
    /// 64-bit device IDs:
    /// <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/include/linux/types.h#L18>.
    /// So this encoding follows the implementation in glibc:
    /// <https://github.com/bminor/glibc/blob/632d895f3e5d98162f77b9c3c1da4ec19968b671/bits/sysmacros.h#L26-L34>.
    pub fn as_encoded_u64(&self) -> u64 {
        let major = self.major() as u64;
        let minor = self.minor() as u64;
        ((major & 0xffff_f000) << 32)
            | ((major & 0x0000_0fff) << 8)
            | ((minor & 0xffff_ff00) << 12)
            | (minor & 0x0000_00ff)
    }
}

/// Add a device node to FS for the device.
///
/// If the parent path is not existing, `mkdir -p` the parent path.
/// This function is used in registering device.
pub fn add_node(device: Arc<dyn Device>, path: &str) -> Result<Path> {
    let mut dev_path = {
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

        match dev_path.lookup(next_name) {
            Ok(next_path) => {
                if path_remain.is_empty() {
                    return_errno_with_message!(Errno::EEXIST, "device node is existing");
                }
                dev_path = next_path;
            }
            Err(_) => {
                if path_remain.is_empty() {
                    // Create the device node
                    dev_path = dev_path.mknod(
                        next_name,
                        InodeMode::from_bits_truncate(0o666),
                        device.clone().into(),
                    )?;
                } else {
                    // Mkdir parent path
                    dev_path = dev_path.new_fs_child(
                        next_name,
                        InodeType::Dir,
                        InodeMode::from_bits_truncate(0o755),
                    )?;
                }
            }
        }
        relative_path = path_remain;
    }

    Ok(dev_path)
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

    let (parent_path, name) = {
        let fs_resolver = FsResolver::new();
        fs_resolver.lookup_dir_and_base_name(&FsPath::try_from(abs_path.as_str()).unwrap())?
    };

    parent_path.unlink(&name)?;
    Ok(())
}
