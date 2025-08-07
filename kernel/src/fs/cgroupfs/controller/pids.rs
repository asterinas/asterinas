// SPDX-License-Identifier: MPL-2.0

use aster_systree::{Error, Result, SysAttrSet, SysAttrSetBuilder, SysPerms, SysStr};
use ostd::mm::{VmReader, VmWriter};

use crate::fs::cgroupfs::controller::CgroupSysNode;

/// The controller responsible for PID in the cgroup subsystem.
///
/// This controller will only provide interfaces in non-root cgroup node.
pub struct PidsController {
    attrs: SysAttrSet,
}

impl PidsController {
    pub(super) fn new() -> Self {
        let mut builder = SysAttrSetBuilder::new();

        builder.add(SysStr::from("pids.max"), SysPerms::DEFAULT_RW_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");
        Self { attrs }
    }
}

impl super::SubControl for PidsController {
    fn attr_set(&self) -> &SysAttrSet {
        &self.attrs
    }

    fn read_attr(
        &self,
        _name: &str,
        _writer: &mut VmWriter,
        _cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(
        &self,
        _name: &str,
        _reader: &mut VmReader,
        _cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        Err(Error::AttributeError)
    }
}
