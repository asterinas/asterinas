// SPDX-License-Identifier: MPL-2.0

pub mod cgroupfs;
pub mod configfs;
pub mod device;
pub mod devpts;
pub mod epoll;
pub mod exfat;
pub mod ext2;
pub mod file_handle;
pub mod file_table;
pub mod inode_handle;
pub mod notify;
pub mod overlayfs;
pub mod path;
pub mod pipe;
pub mod procfs;
pub mod pseudofs;
pub mod ramfs;
pub mod registry;
pub mod rootfs;
pub mod sysfs;
pub mod thread_info;
pub mod tmpfs;
pub mod utils;

use crate::{
    fs::{
        file_table::FdFlags,
        path::{FsPath, PathResolver},
        utils::{AccessMode, OpenArgs, mkmod},
    },
    prelude::*,
};

pub fn init() {
    registry::init();

    sysfs::init();
    procfs::init();
    cgroupfs::init();
    configfs::init();
    ramfs::init();
    tmpfs::init();
    devpts::init();
    pseudofs::init();

    ext2::init();
    exfat::init();
    overlayfs::init();

    path::init();
}

pub fn init_on_each_cpu() {
    procfs::init_on_each_cpu();
}

pub fn init_in_first_kthread(path_resolver: &PathResolver) {
    rootfs::init_in_first_kthread(path_resolver).unwrap();
}

pub fn init_in_first_process(ctx: &Context) {
    let fs = ctx.thread_local.borrow_fs();
    let path_resolver = fs.resolver().read();

    // Initialize the file table for the first process.
    let tty_path = FsPath::try_from("/dev/console").unwrap();
    let stdin = {
        let open_args = OpenArgs::from_modes(AccessMode::O_RDONLY, mkmod!(u+r));
        path_resolver
            .lookup(&tty_path)
            .unwrap()
            .open(open_args)
            .unwrap()
    };
    let stdout = {
        let open_args = OpenArgs::from_modes(AccessMode::O_WRONLY, mkmod!(u+w));
        path_resolver
            .lookup(&tty_path)
            .unwrap()
            .open(open_args)
            .unwrap()
    };
    let stderr = {
        let open_args = OpenArgs::from_modes(AccessMode::O_WRONLY, mkmod!(u+w));
        path_resolver
            .lookup(&tty_path)
            .unwrap()
            .open(open_args)
            .unwrap()
    };

    let mut file_table_ref = ctx.thread_local.borrow_file_table_mut();
    let mut file_table = file_table_ref.unwrap().write();

    file_table.insert(Arc::new(stdin), FdFlags::empty());
    file_table.insert(Arc::new(stdout), FdFlags::empty());
    file_table.insert(Arc::new(stderr), FdFlags::empty());
}
