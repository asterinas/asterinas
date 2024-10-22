// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/cgroup`.
/// See https://man7.org/linux/man-pages/man7/cgroups.7.html for more details.
/// FIXME: Some fields are not implemented yet.
/// Fields:
/// - hierarchy ID: The hierarchy ID of the cgroup. This is a unique identifier for the cgroup hierarchy.
/// - cgroup path: The path of the cgroup in the hierarchy. This is the path to the cgroup within the hierarchy.
/// - controllers: The list of controllers attached to the cgroup. These are the subsystems (e.g., cpu, memory) that are attached to the cgroup.
/// - cgroup tasks: The list of tasks in the cgroup. These are the tasks (threads) that are part of the cgroup.
/// - cgroup.procs: The list of processes in the cgroup. These are the processes that are part of the cgroup.
pub struct CgroupFileOps;

impl CgroupFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for CgroupFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        Ok("0::/\n".to_string().into_bytes())
    }
}
