// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        fs_resolver::{split_path, FsPath},
        path::Dentry,
        utils::{InodeMode, InodeType, Permission},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

pub fn lookup_socket_file(path: &str) -> Result<Dentry> {
    let dentry = {
        let current = current_thread!();
        let current = current.as_posix_thread().unwrap();
        let fs = current.fs().resolver().read();
        let fs_path = FsPath::try_from(path)?;
        fs.lookup(&fs_path)?
    };

    if dentry
        .inode()
        .check_permission(Permission::MAY_READ | Permission::MAY_WRITE)
        .is_err()
    {
        return_errno_with_message!(Errno::EACCES, "the socket file cannot be read or written")
    }

    if dentry.type_() != InodeType::Socket {
        return_errno_with_message!(
            Errno::ECONNREFUSED,
            "the specified file is not a socket file"
        )
    }

    Ok(dentry)
}

pub fn create_socket_file(path: &str) -> Result<Dentry> {
    let (parent_pathname, file_name) = split_path(path);

    let parent = {
        let current = current_thread!();
        let current = current.as_posix_thread().unwrap();
        let fs = current.fs().resolver().read();
        let parent_path = FsPath::try_from(parent_pathname)?;
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
