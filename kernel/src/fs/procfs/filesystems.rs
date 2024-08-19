// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at /proc/filesystems.
pub struct FileSystemsFileOps;

impl FileSystemsFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for FileSystemsFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let mut result = String::new();
        for fs in FILESYSTEM_TYPES.iter() {
            if fs.is_nodev {
                result.push_str(&format!("nodev\t{}\n", fs.name));
            } else {
                result.push_str(&format!("\t{}\n", fs.name));
            }
        }
        Ok(result.into_bytes())
    }
}

lazy_static! {
    static ref FILESYSTEM_TYPES: Vec<FileSystemType> = {
        vec![
            FileSystemType::new("proc", true),
            FileSystemType::new("ramfs", true),
            FileSystemType::new("devpts", true),
            FileSystemType::new("ext2", false),
            FileSystemType::new("exfat", false),
        ]
    };
}

struct FileSystemType {
    name: String,
    is_nodev: bool,
}

impl FileSystemType {
    fn new(name: &str, is_nodev: bool) -> Self {
        FileSystemType {
            name: name.to_string(),
            is_nodev,
        }
    }
}
