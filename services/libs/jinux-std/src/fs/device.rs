use crate::fs::fs_resolver::{FsPath, FsResolver};
use crate::fs::utils::{Dentry, Device, InodeMode, InodeType};
use crate::prelude::*;

/// Add a device node to FS for the device.
///
/// If the parent path is not existing, `mkdir -p` the parent path.
/// This function is used in registering device.
pub fn add_node(device: Arc<dyn Device>, path: &str) -> Result<Arc<Dentry>> {
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
                        device.clone(),
                    )?;
                } else {
                    // Mkdir parent path
                    dentry = dentry.create(
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
