// SPDX-License-Identifier: MPL-2.0

use aster_systree::{Error, Result, SysAttrSet, SysAttrSetBuilder, SysPerms, SysStr};
use ostd::mm::{VmReader, VmWriter};

use crate::fs::cgroupfs::controller::CgroupSysNode;

/// The controller responsible for cpuset in the cgroup subsystem.
pub struct CpuSetController {
    attrs: SysAttrSet,
}

impl CpuSetController {
    pub(super) fn new(is_root: bool) -> Self {
        let mut builder = SysAttrSetBuilder::new();

        if !is_root {
            builder.add(SysStr::from("cpuset.cpus"), SysPerms::DEFAULT_RW_ATTR_PERMS);
            builder.add(SysStr::from("cpuset.mems"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        }

        builder.add(
            SysStr::from("cpuset.cpus.effective"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cpuset.mems.effective"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );

        let attrs = builder.build().expect("Failed to build attribute set");
        Self { attrs }
    }
}

impl super::SubControl for CpuSetController {
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
