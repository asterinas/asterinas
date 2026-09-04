// SPDX-License-Identifier: MPL-2.0

//! The device temporary filesystem.
//!
//! This module implements `devtmpfs` as a singleton filesystem backed by
//! `tmpfs`. Device subsystems submit node-management requests through
//! [`create_node`] and [`delete_node`]; the requests are serialized and handled
//! by the dedicated kernel thread `devtmpfsd`. This gives node operations a
//! single VFS execution context instead of mutating the filesystem tree directly
//! from arbitrary device-registration contexts, and keeps these operations
//! independent of the credentials of the device-registration caller, following
//! Linux's devtmpfs design.
//!
//! Mounting policy follows the selected init path. If an initramfs init is
//! selected, either by `rdinit=` or by the default `/init` lookup, the kernel
//! does not mount devtmpfs automatically; the selected initramfs init program is
//! responsible for mounting it if needed. Otherwise, Asterinas boots from the
//! configured root filesystem and the kernel mounts this singleton on `/dev`
//! during first-process initialization.

mod fs;
mod tree;
mod worker;

pub(in crate::fs) use fs::singleton;
pub(crate) use tree::{DevtmpfsNode, DevtmpfsNodeMeta};
pub(crate) use worker::{create_node, delete_node};

pub(super) fn init() {
    fs::init();
}

pub(super) fn init_in_first_kthread() {
    worker::init_in_first_kthread();
}

#[cfg(ktest)]
mod tests {
    use device_id::{DeviceId, MajorId, MinorId};
    use ostd::prelude::ktest;

    use super::{
        DevtmpfsNode, DevtmpfsNodeMeta, create_node, delete_node, singleton, tree, worker,
    };
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

    fn device_id(major: u16, minor: u32) -> DeviceId {
        DeviceId::new(MajorId::new(major), MinorId::new(minor))
    }

    fn char_node(path: &'static str, major: u16, minor: u32) -> DevtmpfsNode {
        DevtmpfsNode::new(
            DeviceType::Char,
            device_id(major, minor),
            DevtmpfsNodeMeta::with_mode(path, mkmod!(a+rw)).unwrap(),
        )
    }

    #[track_caller]
    fn assert_missing(path: &str) {
        match tree::lookup_path(path) {
            Err(error) => assert_eq!(error.error(), Errno::ENOENT),
            Ok(_) => panic!("path {path} should not exist"),
        }
    }

    fn to_be_revalidated(inode: &dyn Inode) -> bool {
        inode
            .downcast_ref::<RamInode>()
            .unwrap()
            .to_be_revalidated()
    }

    #[ktest]
    fn create_node_marks_ancestor_dirs_and_device_node() {
        worker::init_for_ktest();

        let path = "__devtmpfs_ktest_create/input/event0";
        create_node(char_node(path, 240, 1)).unwrap();

        let top_dir = tree::lookup_path("__devtmpfs_ktest_create").unwrap();
        let input_dir = tree::lookup_path("__devtmpfs_ktest_create/input").unwrap();
        let event_node = tree::lookup_path(path).unwrap();

        assert_eq!(top_dir.type_(), InodeType::Dir);
        assert_eq!(input_dir.type_(), InodeType::Dir);
        assert_eq!(event_node.type_(), InodeType::CharDevice);
        assert!(to_be_revalidated(top_dir.as_ref()));
        assert!(to_be_revalidated(input_dir.as_ref()));
        assert!(to_be_revalidated(event_node.as_ref()));

        let metadata = event_node.metadata().unwrap();
        assert_eq!(metadata.self_dev_id, Some(device_id(240, 1)));
        assert_eq!(metadata.mode.bits(), mkmod!(a+rw).bits());

        delete_node(char_node(path, 240, 1)).unwrap();
        assert_missing("__devtmpfs_ktest_create");
    }

    #[ktest]
    fn delete_node_keeps_nonempty_parent_dir() {
        worker::init_for_ktest();

        let node0 = char_node("__devtmpfs_ktest_nonempty/input/event0", 240, 3);
        let node1 = char_node("__devtmpfs_ktest_nonempty/input/event1", 240, 4);
        create_node(node0).unwrap();
        create_node(node1).unwrap();

        delete_node(char_node("__devtmpfs_ktest_nonempty/input/event0", 240, 3)).unwrap();
        assert_missing("__devtmpfs_ktest_nonempty/input/event0");
        assert!(tree::lookup_path("__devtmpfs_ktest_nonempty/input").is_ok());
        assert!(tree::lookup_path("__devtmpfs_ktest_nonempty/input/event1").is_ok());

        delete_node(char_node("__devtmpfs_ktest_nonempty/input/event1", 240, 4)).unwrap();
        assert_missing("__devtmpfs_ktest_nonempty");
    }

    #[ktest]
    fn delete_node_keeps_unmarked_node() {
        worker::init_for_ktest();

        let path = "__devtmpfs_ktest_unmarked";
        let root = singleton().root_inode();
        let root_dentry = crate::fs::vfs::path::Dentry::new_root(root.clone());
        root.mknod(
            &root_dentry,
            path,
            mkmod!(a+rw),
            MknodType::CharDevice(device_id(240, 5).as_encoded_u64()),
        )
        .unwrap();
        let inode = root.lookup(path).unwrap();
        assert!(!to_be_revalidated(inode.as_ref()));

        delete_node(char_node(path, 240, 5)).unwrap();

        let inode = root.lookup(path).unwrap();
        assert!(!to_be_revalidated(inode.as_ref()));
        let inode_dentry = root_dentry
            .as_dir_dentry_or_err()
            .unwrap()
            .lookup_child(path)
            .unwrap();
        root.unlink(&inode_dentry).unwrap();
    }

    #[ktest]
    fn delete_node_keeps_device_id_mismatch() {
        worker::init_for_ktest();

        let path = "__devtmpfs_ktest_mismatch";
        create_node(char_node(path, 240, 6)).unwrap();

        delete_node(char_node(path, 240, 7)).unwrap();

        let inode = tree::lookup_path(path).unwrap();
        assert!(to_be_revalidated(inode.as_ref()));
        assert_eq!(
            inode.metadata().unwrap().self_dev_id,
            Some(device_id(240, 6))
        );
        delete_node(char_node(path, 240, 6)).unwrap();
    }
}
