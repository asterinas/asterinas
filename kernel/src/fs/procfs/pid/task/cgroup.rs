// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_systree::SysObj;

use crate::{
    fs::{
        cgroupfs,
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/cgroup`.
pub struct CgroupOps(Arc<Process>);

impl CgroupOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CgroupOps {
    fn data(&self) -> Result<Vec<u8>> {
        let path = self
            .0
            .cgroup()
            .get()
            .map(|cgroup| cgroup.path())
            .unwrap_or_else(|| cgroupfs::singleton().systree_root().path());

        let data = format!("{}{}\n", "0::", path);
        Ok(data.into_bytes())
    }
}
