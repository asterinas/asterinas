// SPDX-License-Identifier: MPL-2.0

use aster_systree::{Error, Result, SysAttrSet, SysAttrSetBuilder, SysPerms, SysStr};
use ostd::mm::{VmReader, VmWriter};

use crate::fs::cgroupfs::controller::CgroupSysNode;

/// The controller responsible for memory management in the cgroup subsystem.
///
/// Note that even if the controller is inactive, it still provides some interfaces
/// like "memory.pressure" for usage.
pub struct MemoryController {
    attrs: SysAttrSet,
}

impl MemoryController {
    pub(super) fn new(is_active: bool, is_root: bool) -> Self {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(
            SysStr::from("memory.pressure"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        if is_active {
            builder.add(SysStr::from("memory.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);
            if !is_root {
                builder.add(SysStr::from("memory.max"), SysPerms::DEFAULT_RO_ATTR_PERMS);
                builder.add(
                    SysStr::from("memory.events"),
                    SysPerms::DEFAULT_RO_ATTR_PERMS,
                );
            }
        }

        let attrs = builder.build().expect("Failed to build attribute set");
        Self { attrs }
    }
}

impl super::SubControl for MemoryController {
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
