// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr, MAX_ATTR_SIZE};
use aster_util::printer::VmPrinter;
use ostd::mm::{VmReader, VmWriter};

use crate::util::ReadCString;

/// A sub-controller responsible for PID resource management in the cgroup subsystem.
///
/// This controller will only provide interfaces in non-root cgroup nodes.
pub struct PidsController {
    max_pid: AtomicU32,
}

impl PidsController {
    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        if !is_root {
            builder.add(SysStr::from("pids.max"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        }
    }
}

impl super::SubControl for PidsController {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "pids.max" => {
                let max_pid = self.max_pid.load(Ordering::Relaxed);
                if max_pid == u32::MAX {
                    writeln!(printer, "max")?;
                } else {
                    writeln!(printer, "{}", max_pid)?;
                }
            }
            _ => return Err(Error::AttributeError),
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
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
                    u32::MAX
                } else {
                    value.parse::<u32>().map_err(|_| Error::InvalidOperation)?
                };

                log::warn!("The PIDs controller does not enforce PID limits yet.");
                self.max_pid.store(value, Ordering::Relaxed);

                Ok(len)
            }
            _ => Err(Error::AttributeError),
        }
    }
}

impl super::SubControlStatic for PidsController {
    fn new(_is_root: bool) -> Self {
        Self {
            max_pid: AtomicU32::new(u32::MAX),
        }
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::Pids
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.pids.read().get().clone()
    }
}
