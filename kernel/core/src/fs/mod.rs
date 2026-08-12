// SPDX-License-Identifier: MPL-2.0

pub(crate) mod file;
mod fs_impls;
pub(crate) mod initramfs;
pub(crate) mod pipe;
pub(crate) mod rootfs;
pub(crate) mod thread_info;
pub(crate) mod utils;
pub(crate) mod vfs;

pub(crate) use fs_impls::{
    cgroupfs, configfs, devpts, exfat, ext2, procfs, pseudofs, ramfs, sysfs, tmpfs,
};

use crate::{
    fs::{
        file::{AccessMode, OpenArgs, file_table::FdFlags, mkmod},
        vfs::path::{FsPath, PathResolver},
    },
    prelude::*,
};

pub(crate) fn init() {
    vfs::init();
    fs_impls::init();
}

pub(crate) fn init_on_each_cpu() {
    fs_impls::init_on_each_cpu();
}

pub(crate) fn init_in_first_kthread(path_resolver: &PathResolver) {
    initramfs::init_in_first_kthread(path_resolver).unwrap();
}

pub(crate) fn init_in_first_process(ctx: &Context) {
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
