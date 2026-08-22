// SPDX-License-Identifier: MPL-2.0

//! Filesystem-tree operations for creating and deleting devtmpfs nodes.

use super::DevtmpfsNode;
use crate::{
    device::DeviceType,
    fs::{
        file::{InodeType, mkmod},
        fs_impls::ramfs::RamInode,
        vfs::{
            file_system::FileSystem,
            inode::{Inode, MknodType},
        },
    },
    prelude::*,
};

pub(super) fn create_node(node: &DevtmpfsNode) -> Result<()> {
    let (parent_path, node_name) = split_parent_and_basename(node.meta.path()).unwrap();
    let parent_inode = lookup_or_create_path(parent_path)?;
    create_device_node(parent_inode.as_ref(), node_name, node)
}

pub(super) fn delete_node(node: &DevtmpfsNode) -> Result<()> {
    let (parent_path, node_name) = split_parent_and_basename(node.meta.path()).unwrap();
    let parent_inode = lookup_path(parent_path)?;
    let parent_ram_inode = parent_inode.downcast_ref::<RamInode>().unwrap();

    if parent_ram_inode.unlink_if(node_name, |inode| matches_device(inode, node))? {
        remove_empty_parent_dirs(parent_path);
    }
    Ok(())
}

pub(super) fn lookup_path(path: &str) -> Result<Arc<dyn Inode>> {
    let mut current = super::fs::singleton().root_inode();
    for name in path.split('/').filter(|name| !name.is_empty()) {
        current = current.lookup(name)?;
    }
    Ok(current)
}

fn lookup_or_create_path(path: &str) -> Result<Arc<dyn Inode>> {
    let mut current = super::fs::singleton().root_inode();
    for name in path.split('/').filter(|name| !name.is_empty()) {
        current = lookup_or_create_dir(current.as_ref(), name)?;
    }
    Ok(current)
}

fn lookup_or_create_dir(parent_inode: &dyn Inode, name: &str) -> Result<Arc<dyn Inode>> {
    let parent_ram_inode = parent_inode.downcast_ref::<RamInode>().unwrap();

    loop {
        let error = match parent_ram_inode.mkdir_with_revalidation(name, mkmod!(a+rx, u+w)) {
            Ok(inode) => return Ok(inode),
            Err(error) => error,
        };
        if error.error() != Errno::EEXIST {
            return Err(error);
        }

        match parent_inode.lookup(name) {
            Ok(inode) if inode.type_() == InodeType::Dir => return Ok(inode),
            Ok(_) => {
                return_errno_with_message!(Errno::ENOTDIR, "the parent path is not a directory")
            }
            Err(error) if error.error() == Errno::ENOENT => continue,
            Err(error) => return Err(error),
        }
    }
}

fn create_device_node(parent_inode: &dyn Inode, name: &str, node: &DevtmpfsNode) -> Result<()> {
    let rdev = node.device_id.as_encoded_u64();
    let mknod_type = match node.device_type {
        DeviceType::Block => MknodType::BlockDevice(rdev),
        DeviceType::Char => MknodType::CharDevice(rdev),
    };

    let parent_ram_inode = parent_inode.downcast_ref::<RamInode>().unwrap();
    parent_ram_inode.mknod_with_revalidation(name, node.meta.mode(), mknod_type)?;
    Ok(())
}

fn remove_empty_parent_dirs(path: &str) {
    let mut path = path;

    while let Some((parent_path, name)) = split_parent_and_basename(path) {
        let parent_inode = match lookup_path(parent_path) {
            Ok(inode) => inode,
            Err(_) => break,
        };
        let parent_ram_inode = parent_inode.downcast_ref::<RamInode>().unwrap();
        match parent_ram_inode.rmdir_if(name, |inode| inode.to_be_revalidated()) {
            Ok(true) => {}
            _ => break,
        }
        path = parent_path;
    }
}

fn matches_device(inode: &RamInode, node: &DevtmpfsNode) -> bool {
    if !inode.to_be_revalidated() {
        return false;
    }

    let expected_type = match node.device_type {
        DeviceType::Block => InodeType::BlockDevice,
        DeviceType::Char => InodeType::CharDevice,
    };

    inode.type_() == expected_type && inode.metadata().unwrap().self_dev_id == Some(node.device_id)
}

fn split_parent_and_basename(path: &str) -> Option<(&str, &str)> {
    if path.is_empty() {
        return None;
    }

    path.rsplit_once('/').map_or_else(
        || Some(("", path)),
        |(parent, basename)| (!basename.is_empty()).then_some((parent, basename)),
    )
}
