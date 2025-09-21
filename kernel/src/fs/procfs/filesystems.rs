// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        registry::FsProperties,
        utils::{mkmod, Inode},
    },
    prelude::*,
};

/// Represents the inode at /proc/filesystems.
pub struct FileSystemsFileOps;

impl FileSystemsFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/filesystems.c#L259>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for FileSystemsFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let data = crate::fs::registry::with_iter(|iter| {
            let mut result = String::new();
            for (fs_name, fs_type) in iter {
                if fs_type.properties().contains(FsProperties::NEED_DISK) {
                    result.push_str(&format!("\t{}\n", fs_name));
                } else {
                    result.push_str(&format!("nodev\t{}\n", fs_name));
                }
            }

            result.into_bytes()
        });

        Ok(data)
    }
}
