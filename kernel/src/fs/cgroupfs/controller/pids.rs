// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::{
    Error, Result, SysAttrSet, SysAttrSetBuilder, SysPerms, SysStr, MAX_ATTR_SIZE,
};
use aster_util::printer::VmPrinter;
use ostd::mm::{VmReader, VmWriter};

use crate::{fs::cgroupfs::controller::CgroupSysNode, util::ReadCString};

/// The controller responsible for PID in the cgroup subsystem.
///
/// This controller will only provide interfaces in non-root cgroup nodes.
pub struct PidsController {
    max_pid: AtomicUsize,
    attrs: SysAttrSet,
}

impl PidsController {
    pub(super) fn new() -> Self {
        let mut builder = SysAttrSetBuilder::new();

        builder.add(SysStr::from("pids.max"), SysPerms::DEFAULT_RW_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");
        Self {
            max_pid: AtomicUsize::new(usize::MAX),
            attrs,
        }
    }
}

impl super::SubControl for PidsController {
    fn attr_set(&self) -> &SysAttrSet {
        &self.attrs
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
        _cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "pids.max" => {
                let max_pid = self.max_pid.load(Ordering::Relaxed);
                if max_pid == usize::MAX {
                    writeln!(printer, "max")?;
                } else {
                    writeln!(printer, "{}", max_pid)?;
                }
            }
            _ => return Err(Error::AttributeError),
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(
        &self,
        name: &str,
        reader: &mut VmReader,
        _cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        match name {
            "pids.max" => {
                let (content, len) = reader
                    .read_cstring_until_end(MAX_ATTR_SIZE)
                    .map_err(|_| Error::PageFault)?;
                let value = content
                    .to_str()
                    .map_err(|_| Error::InvalidOperation)?
                    .trim();
                let value = if value == "max" {
                    usize::MAX
                } else {
                    value
                        .parse::<usize>()
                        .map_err(|_| Error::InvalidOperation)?
                };

                self.max_pid.store(value, Ordering::Relaxed);

                Ok(len)
            }
            _ => Err(Error::AttributeError),
        }
    }
}
