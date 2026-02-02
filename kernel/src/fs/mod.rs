// SPDX-License-Identifier: MPL-2.0

pub mod epoll;
pub mod file;
pub mod fs_impls;
pub mod pipe;
pub mod rootfs;
pub mod thread_info;
pub mod utils;
pub mod vfs;

pub use fs_impls::*;

use crate::{
    fs::{
        file::{AccessMode, OpenArgs, file_table::FdFlags, mkmod},
        vfs::path::{FsPath, PathResolver},
    },
    prelude::*,
};

pub fn init() {
    vfs::init();
    fs_impls::init();
}

pub fn init_on_each_cpu() {
    fs_impls::init_on_each_cpu();
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
