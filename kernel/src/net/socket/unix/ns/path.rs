// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use crate::{
    fs::{
        file::{InodeType, Permission, mkmod},
        vfs::path::{FsPath, Path},
    },
    prelude::*,
};

pub fn lookup_socket_file(path: &str) -> Result<Path> {
    let path = {
        let current = Task::current().unwrap();
        let fs_ref = current.as_thread_local().unwrap().borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        let fs_path = FsPath::try_from(path)?;
        path_resolver.lookup(&fs_path)?
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
    let result = (|| {
        let current = Task::current().unwrap();
        let fs_ref = current.as_thread_local().unwrap().borrow_fs();
        let path_resolver = fs_ref.resolver().read();

        let fs_path = FsPath::try_from(path_name)?;
        let (parent, file_name) = path_resolver
            .lookup_unresolved_no_follow(&fs_path)?
            .into_parent_and_filename()?;

        parent.new_fs_child(&file_name, InodeType::Socket, mkmod!(u+rw))
    })();

    result.map_err(|err| {
        if err.error() == Errno::EEXIST {
            Error::with_message(Errno::EADDRINUSE, "the socket file already exists")
        } else {
            err
        }
    })
}
