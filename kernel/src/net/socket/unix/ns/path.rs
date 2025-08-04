// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use crate::{
    fs::{
        fs_resolver::{split_path, FsPath},
        path::Path,
        utils::{InodeMode, InodeType, Permission},
    },
    prelude::*,
};

pub fn lookup_socket_file(path: &str) -> Result<Path> {
    let path = {
        let current = Task::current().unwrap();
        let fs_ref = current.as_thread_local().unwrap().borrow_fs();
        let fs = fs_ref.resolver().read();
        let fs_path = FsPath::try_from(path)?;
        fs.lookup(&fs_path)?
    };

    if path
        .inode()
        .check_permission(Permission::MAY_READ | Permission::MAY_WRITE)
        .is_err()
    {
        return_errno_with_message!(Errno::EACCES, "the socket file cannot be read or written")
    }

    if path.type_() != InodeType::Socket {
        return_errno_with_message!(
            Errno::ECONNREFUSED,
            "the specified file is not a socket file"
        )
    }

    Ok(path)
}

pub fn create_socket_file(path_name: &str) -> Result<Path> {
    let (parent_path_name, file_name) = split_path(path_name);

    let parent = {
        let current = Task::current().unwrap();
        let fs_ref = current.as_thread_local().unwrap().borrow_fs();
        let fs = fs_ref.resolver().read();
        let parent_path = FsPath::try_from(parent_path_name)?;
        fs.lookup(&parent_path)?
    };

    parent
        .new_fs_child(
            file_name,
            InodeType::Socket,
            InodeMode::S_IRUSR | InodeMode::S_IWUSR,
        )
        .map_err(|err| {
            if err.error() == Errno::EEXIST {
                Error::with_message(Errno::EADDRINUSE, "the socket file already exists")
            } else {
                err
            }
        })
}
