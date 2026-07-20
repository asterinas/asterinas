// SPDX-License-Identifier: MPL-2.0

pub mod file;
mod fs_impls;
pub mod pipe;
pub mod rootfs;
pub mod thread_info;
pub mod utils;
pub mod vfs;

pub use fs_impls::{
    cgroupfs, configfs, devpts, devtmpfs, exfat, ext2, procfs, pseudofs, ramfs, sysfs, tmpfs,
};

use crate::{
    fs::{
        file::{AccessMode, InodeType, OpenArgs, file_table::FdFlags, mkmod},
        vfs::path::{FsPath, Path, PathResolver, PerMountFlags},
    },
    init,
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
    fs_impls::init_in_first_kthread();
    rootfs::init_in_first_kthread(path_resolver).unwrap();
}

pub fn init_in_first_process(ctx: &Context) {
    let fs = ctx.thread_local.borrow_fs();
    let path_resolver = fs.resolver().read();

    if init::booted_from_rootfs() {
        let dev_path = lookup_or_create_dev(&path_resolver).unwrap();
        dev_path
            .mount(
                devtmpfs::singleton().clone(),
                PerMountFlags::default(),
                Some("devtmpfs".to_string()),
                ctx,
            )
            .unwrap();
    }

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

fn lookup_or_create_dev(path_resolver: &PathResolver) -> Result<Path> {
    match path_resolver.lookup(&FsPath::try_from("/dev")?) {
        Err(error) if error.error() == Errno::ENOENT => {
            path_resolver
                .root()
                .new_fs_child("dev", InodeType::Dir, mkmod!(a+rx, u+w))
        }
        result => result,
    }
}
