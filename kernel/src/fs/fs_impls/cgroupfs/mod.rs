// SPDX-License-Identifier: MPL-2.0

pub use cgroup_ns::CgroupNamespace;
pub use controller::cpu::{CpuStatKind, charge_cpu_time};
use fs::CgroupFsType;
pub(in crate::fs) use systree_node::CgroupSystem;
pub use systree_node::{CgroupMembership, CgroupNode, CgroupSysNode};

mod cgroup_ns;
mod controller;
mod fs;
mod inode;
mod systree_node;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub(super) fn init() {
    crate::fs::vfs::registry::register(&CgroupFsType).unwrap();
}
