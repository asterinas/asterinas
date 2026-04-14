// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr};
use aster_util::{per_cpu_counter::PerCpuCounter, printer::VmPrinter};
use ostd::{
    cpu::CpuId,
    mm::{VmReader, VmWriter},
    task::atomic_mode::AsAtomicModeGuard,
    timer::Jiffies,
};

use crate::{
    fs::cgroupfs::systree_node::{CgroupSysNode, CgroupSystem},
    process::Process,
};

/// A sub-controller responsible for CPU resource management in the cgroup subsystem.
///
/// This controller always exists so that `cpu.stat` remains readable even before `+cpu`
/// is enabled. When inactive, only the base usage fields are exposed. The `user` and
/// `system` counters store CPU time in units of one [`Jiffies`] per increment, and
/// `usage_usec` is derived from their sum when `cpu.stat` is read.
pub struct CpuController {
    is_enabled: AtomicBool,
    user: PerCpuCounter,
    system: PerCpuCounter,
}

/// Specifies the cgroup CPU sub-controller receives one [`Jiffies`] of charge.
#[derive(Clone, Copy)]
pub enum CpuStatKind {
    /// Charges one [`Jiffies`] to `user_usec`.
    User,
    /// Charges one [`Jiffies`] to `system_usec`.
    System,
}

impl CpuController {
    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, _is_root: bool) {
        builder.add(SysStr::from("cpu.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);
    }

    fn new(is_enabled: bool) -> Self {
        Self {
            is_enabled: AtomicBool::new(is_enabled),
            user: PerCpuCounter::new(),
            system: PerCpuCounter::new(),
        }
    }

    fn write_cpu_stat_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        /// Converts a per-CPU counter expressed in jiffies to microseconds.
        fn counter_usec(counter: &PerCpuCounter) -> u64 {
            let jiffies = u64::try_from(counter.sum_all_cpus()).unwrap_or(u64::MAX);
            u64::try_from(Jiffies::new(jiffies).as_duration().as_micros()).unwrap_or(u64::MAX)
        }

        let mut printer = VmPrinter::new_skip(writer, offset);

        let user_usec = counter_usec(&self.user);
        let system_usec = counter_usec(&self.system);
        writeln!(
            printer,
            "usage_usec {}",
            user_usec.saturating_add(system_usec)
        )?;
        writeln!(printer, "user_usec {}", user_usec)?;
        writeln!(printer, "system_usec {}", system_usec)?;

        if self.is_enabled.load(Ordering::Relaxed) {
            // TODO: Support CPU bandwidth control statistics. These fields are reported as `0`
            // for now because the cgroup CPU sub-controller does not yet implement throttling or
            // burst accounting.
            writeln!(printer, "nr_periods 0")?;
            writeln!(printer, "nr_throttled 0")?;
            writeln!(printer, "throttled_usec 0")?;
            writeln!(printer, "nr_bursts 0")?;
            writeln!(printer, "burst_usec 0")?;
        }

        Ok(printer.bytes_written())
    }

    pub(super) fn enable(&self) {
        self.is_enabled.store(true, Ordering::Relaxed);
    }

    pub(super) fn disable(&self) {
        self.is_enabled.store(false, Ordering::Relaxed);
    }

    /// Accounts one [`Jiffies`] of CPU time on `cpu`.
    fn account(&self, cpu: CpuId, stat_kind: CpuStatKind) {
        match stat_kind {
            CpuStatKind::User => self.user.add_on_cpu(cpu, 1),
            CpuStatKind::System => self.system.add_on_cpu(cpu, 1),
        }
    }
}

impl super::SubControl for CpuController {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if name != "cpu.stat" {
            return Err(Error::AttributeError);
        }

        self.write_cpu_stat_at(offset, writer)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

impl super::SubControlStatic for CpuController {
    fn new(is_root: bool, is_active: bool) -> Self {
        Self::new(is_root || is_active)
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::Cpu
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.cpu.read().get().clone()
    }
}

impl super::SubController<CpuController> {
    pub(super) fn enable(&self) {
        self.inner.as_ref().unwrap().enable();
    }

    pub(super) fn disable(&self) {
        self.inner.as_ref().unwrap().disable();
    }

    /// Accounts one [`Jiffies`] of CPU time for this cgroup and all of its ancestors.
    fn account_hierarchy(&self, stat_kind: CpuStatKind) {
        // This is race-free because `charge_cpu_time` holds `cgroup_guard` when
        // invoking this method.
        let cpu = CpuId::current_racy();

        let mut current = Some(self);
        while let Some(node) = current {
            node.inner.as_ref().unwrap().account(cpu, stat_kind);
            current = node.parent.as_deref();
        }
    }
}

impl super::Controller {
    /// Charges one [`Jiffies`] in the CPU sub-controller hierarchy.
    fn charge_cpu_time<G: AsAtomicModeGuard + ?Sized>(&self, guard: &G, stat_kind: CpuStatKind) {
        self.cpu.read_with(guard).account_hierarchy(stat_kind);
    }
}

/// Charges one [`Jiffies`] of CPU time to `process`'s cgroup hierarchy.
///
/// If `process` is not attached to a non-root cgroup, the charge is applied to the root cgroup.
pub fn charge_cpu_time(process: &Process, stat_kind: CpuStatKind) {
    let cgroup_guard = process.cgroup();

    if let Some(cgroup) = cgroup_guard.get() {
        cgroup
            .controller()
            .charge_cpu_time(&cgroup_guard, stat_kind);
    } else {
        CgroupSystem::singleton()
            .controller()
            .charge_cpu_time(&cgroup_guard, stat_kind);
    }
}
