// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use aster_systree::{Error, MAX_ATTR_SIZE, Result, SysAttrSetBuilder, SysPerms, SysStr};
use aster_util::printer::VmPrinter;
use ostd::mm::{VmReader, VmWriter};

use crate::{process::posix_thread::PID_MAX, util::ReadCString};

/// A sub-controller responsible for PID resource management in the cgroup subsystem.
///
/// This controller will only provide interfaces in non-root cgroup nodes.
pub struct PidsController {
    /// The maximum number of processes allowed.
    max_pids: AtomicU32,
    /// The current number of processes in this cgroup's subtree.
    current_count: AtomicU32,
    /// The peak number of processes ever observed in this cgroup's subtree.
    peak_count: AtomicU32,
}

impl PidsController {
    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        if !is_root {
            builder.add(SysStr::from("pids.max"), SysPerms::DEFAULT_RW_ATTR_PERMS);
            builder.add(
                SysStr::from("pids.current"),
                SysPerms::DEFAULT_RO_ATTR_PERMS,
            );
            builder.add(SysStr::from("pids.peak"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        }
    }

    /// Sets the initial process count during sub-controller activation.
    ///
    /// When the pids sub-controller is activated on a cgroup that already
    /// has processes, this method initializes `current_count` and
    /// `peak_count` to reflect the existing state.
    pub(super) fn init_count(&self, count: u32) {
        self.current_count.store(count, Ordering::Relaxed);
        self.peak_count.store(count, Ordering::Relaxed);
    }

    /// Charges one process without limit checking.
    pub(super) fn charge(&self) {
        let new_count = self.current_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak_count.fetch_max(new_count, Ordering::Relaxed);
    }

    /// Tries to charge one process, enforcing the `pids.max` limit.
    ///
    /// Returns `true` if the charge succeeded (limit not exceeded).
    /// Returns `false` if the limit would be exceeded; the charge is rolled back.
    pub(super) fn try_charge(&self) -> bool {
        let new_count = self.current_count.fetch_add(1, Ordering::Relaxed) + 1;
        let max = self.max_pids.load(Ordering::Relaxed);
        if new_count > max {
            self.current_count.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        self.peak_count.fetch_max(new_count, Ordering::Relaxed);
        true
    }

    /// Uncharges one process.
    pub(super) fn uncharge(&self) {
        let old_count = self.current_count.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old_count > 0, "pids current count underflow");
    }
}

impl super::SubControl for PidsController {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "pids.max" => {
                let max_pids = self.max_pids.load(Ordering::Relaxed);
                if max_pids == u32::MAX {
                    writeln!(printer, "max")?;
                } else {
                    writeln!(printer, "{}", max_pids)?;
                }
            }
            "pids.current" => {
                let current = self.current_count.load(Ordering::Relaxed);
                writeln!(printer, "{}", current)?;
            }
            "pids.peak" => {
                let peak = self.peak_count.load(Ordering::Relaxed);
                writeln!(printer, "{}", peak)?;
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
                } else if let Ok(value) = value.parse::<u32>() {
                    if value >= PID_MAX {
                        return Err(Error::InvalidOperation);
                    }
                    value
                } else {
                    return Err(Error::InvalidOperation);
                };

                self.max_pids.store(value, Ordering::Relaxed);

                Ok(len)
            }
            _ => Err(Error::AttributeError),
        }
    }
}

impl super::SubControlStatic for PidsController {
    fn new(_is_root: bool) -> Self {
        Self {
            max_pids: AtomicU32::new(u32::MAX),
            current_count: AtomicU32::new(0),
            peak_count: AtomicU32::new(0),
        }
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::Pids
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.pids.read().get().clone()
    }
}

/// Hierarchical pids charge/uncharge operations.
impl super::SubController<PidsController> {
    /// Charges one process across the hierarchy without limit checking.
    ///
    /// This is used for explicit migration (writing to `cgroup.procs`),
    /// where `pids.max` is not enforced per cgroupv2 semantics.
    ///
    /// Reference: <https://docs.kernel.org/admin-guide/cgroup-v2.html#pid>
    pub(super) fn charge_hierarchy(&self) {
        let mut current = Some(self);
        while let Some(node) = current {
            if let Some(ref inner) = node.inner {
                inner.charge();
            }
            current = node.parent.as_deref();
        }
    }

    /// Tries to charge one process across the hierarchy with limit checking.
    ///
    /// Walks from this cgroup upward to the root. At each level where the
    /// pids sub-controller is active, the counter is incremented and checked
    /// against `pids.max`. If any level exceeds its limit, all previously
    /// charged levels are rolled back and `Err` is returned.
    ///
    /// This is used at fork time where `pids.max` must be enforced.
    pub(super) fn try_charge_hierarchy(&self) -> Result<()> {
        let mut charged: Vec<&PidsController> = Vec::new();
        let mut current = Some(self);

        while let Some(node) = current {
            if let Some(ref inner) = node.inner {
                if !inner.try_charge() {
                    for pid_controller in charged {
                        pid_controller.uncharge();
                    }
                    return Err(Error::ResourceUnavailable);
                }
                charged.push(inner);
            }
            current = node.parent.as_deref();
        }

        Ok(())
    }

    /// Uncharges one process across the hierarchy.
    pub fn uncharge_hierarchy(&self) {
        let mut current = Some(self);
        while let Some(node) = current {
            if let Some(ref inner) = node.inner {
                inner.uncharge();
            }
            current = node.parent.as_deref();
        }
    }
}
