// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use x86::msr::{IA32_VMX_BASIC, rdmsr};

use super::{VMX_GUARD_STATE, VmxGuard, instructions};
use crate::{
    Error,
    cpu::{CpuId, CpuSet, PinCurrentCpu},
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    mm::{Frame, FrameAllocOptions, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, SpinLock},
    task,
};

/// An exclusively owned VMCS.
///
/// Callers should disable preemption across loading and field access.
///
/// # Panics
///
/// Local IRQs must be enabled when dropping a VMCS.
pub(crate) struct Vmcs {
    region: Arc<VmcsRegion>,
}

impl Vmcs {
    /// Allocates and initializes an inactive VMCS.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn new(_vmx_guard: &VmxGuard) -> Result<Self> {
        let frame = FrameAllocOptions::new().alloc_frame_with(VmcsRegionMeta)?;
        let _irq_guard = irq::disable_local();
        // SAFETY: The VMX guard keeps all CPUs in VMX operation.
        let basic = unsafe { rdmsr(IA32_VMX_BASIC) };
        // OSTD's linear mapping uses write-back memory. See Intel SDM,
        // Vol. 3D, Appendix A.1, for the VMCS memory-type requirement.
        const VMCS_MEMORY_TYPE_WB: u64 = 6;
        if (basic >> 50) & 0xf != VMCS_MEMORY_TYPE_WB {
            return Err(Error::NotEnoughResources);
        }

        let revision_id = basic as u32 & 0x7fff_ffff;
        // SAFETY: This newly allocated typed frame is exclusively owned here.
        unsafe { (paddr_to_vaddr(frame.paddr()) as *mut u32).write(revision_id) };

        Ok(Self {
            region: Arc::new(VmcsRegion {
                frame,
                active_cpu: SpinLock::new(None),
            }),
        })
    }

    /// Makes this VMCS current, clearing it on its previous CPU if necessary.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled if the VMCS needs to migrate from another CPU.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn load(&mut self, vmx_guard: &VmxGuard) -> Result<()> {
        LocalVmcsState::activate(&self.region, vmx_guard)
    }

    /// Reads a field, returning an error if this VMCS is not current on this CPU.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn read(&self, field: u32) -> Result<usize> {
        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let local = local.lock();
        if local.current != Some(self.region.frame.paddr()) {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The current entry identifies this retained, active VMCS, so
        // VMX is enabled. IRQs exclude local switching and shutdown; the shared
        // borrow excludes migration and destruction during this read.
        unsafe { instructions::vmread(field, &irq_guard) }
    }

    /// Writes a field, returning an error if this VMCS is not current on this CPU.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn write(&mut self, field: u32, value: usize) -> Result<()> {
        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let local = local.lock();
        if local.current != Some(self.region.frame.paddr()) {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The current entry and IRQ guard keep this VMCS current and
        // VMX enabled as in `read`. The exclusive borrow serializes writes.
        unsafe { instructions::vmwrite(field, value, &irq_guard) }
    }
}

impl Drop for Vmcs {
    fn drop(&mut self) {
        assert!(crate::arch::irq::is_local_enabled());
        let _lifecycle = VMX_GUARD_STATE.lock();
        if let Err(err) = LocalVmcsState::deactivate(&self.region) {
            // The active set still owns the region and shutdown will retry.
            error!("failed to clear VMCS during destruction: {:?}", err);
        }
    }
}

struct VmcsRegionMeta;
crate::impl_frame_meta_for!(VmcsRegionMeta);

struct VmcsRegion {
    frame: Frame<VmcsRegionMeta>,
    active_cpu: SpinLock<Option<CpuId>, LocalIrqDisabled>,
}

pub(super) struct LocalVmcsState {
    current: Option<Paddr>,
    active: BTreeMap<Paddr, Arc<VmcsRegion>>,
    // The target VMCS regions to be cleared on remote CPU.
    pending_clear: Vec<Paddr>,
}

cpu_local! {
    static LOCAL_VMCS_STATE: SpinLock<LocalVmcsState, LocalIrqDisabled> =
        SpinLock::new(LocalVmcsState::new());
}

impl LocalVmcsState {
    const fn new() -> Self {
        Self {
            current: None,
            active: BTreeMap::new(),
            pending_clear: Vec::new(),
        }
    }

    // The exclusive Vmcs borrow in `load` serializes activation and migration.
    fn activate(region: &Arc<VmcsRegion>, _vmx_guard: &VmxGuard) -> Result<()> {
        let preempt_guard = task::disable_preempt();
        let cpu = preempt_guard.current_cpu();
        let active_cpu = *region.active_cpu.lock();
        if active_cpu.is_some_and(|active_cpu| active_cpu != cpu) {
            Self::deactivate(region)?;
        }

        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let mut local = local.lock();
        let paddr = region.frame.paddr();
        if local.current == Some(paddr) {
            return Ok(());
        }

        let was_active = local.active.contains_key(&paddr);
        if !was_active {
            if active_cpu.is_none() {
                // SAFETY:
                //
                // 1. `_vmx_guard` keeps this CPU in VMX root operation.
                // 2. The VMCS region is initialized.
                unsafe { instructions::vmclear(paddr, &irq_guard) }?;
            }
            local.active.insert(paddr, region.clone());
        }

        // SAFETY:
        //
        // 1. `_vmx_guard` keeps this CPU in VMX root operation.
        // 2. The region is initialized. `Self::deactivate(region)?;` makes it inactive on other CPUs.
        if let Err(err) = unsafe { instructions::vmptrld(paddr, &irq_guard) } {
            if !was_active {
                local.active.remove(&paddr);
            }
            return Err(err);
        }
        local.current = Some(paddr);
        *region.active_cpu.lock() = Some(cpu);
        Ok(())
    }

    fn deactivate(region: &VmcsRegion) -> Result<()> {
        let active_cpu = *region.active_cpu.lock();
        let Some(cpu) = active_cpu else {
            return Ok(());
        };
        let preempt_guard = task::disable_preempt();
        let paddr = region.frame.paddr();
        if cpu == preempt_guard.current_cpu() {
            let irq_guard = irq::disable_local();
            // SAFETY:
            // 1. This is the current CPU's state. The caller's VMX guard or
            //    lifecycle lock excludes shutdown.
            // 2. `paddr` comes from the typed VMCS region active on this CPU;
            //    exclusive Vmcs access and disabled IRQs prevent concurrent
            //    access to the region.
            return unsafe {
                LOCAL_VMCS_STATE
                    .get_with(&irq_guard)
                    .lock()
                    .clear(paddr, &irq_guard)
            };
        }

        assert!(crate::arch::irq::is_local_enabled());
        LOCAL_VMCS_STATE
            .get_on_cpu(cpu)
            .lock()
            .pending_clear
            .push(paddr);
        // No spinlock is held while sending or waiting. The callback only takes
        // the target's local-state lock, never the VMX lifecycle mutex.
        crate::smp::inter_processor_call(&CpuSet::from(cpu), || {
            let irq_guard = irq::disable_local();
            let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
            let mut local = local.lock();
            while let Some(paddr) = local.pending_clear.pop() {
                // SAFETY:
                // 1. This is the current CPU's state. The caller keeps its VMX
                //    guard or lifecycle lock until this IPI completes.
                // 2. Each queued address identifies a retained VMCS region active
                //    on this CPU. The caller's exclusive Vmcs access and disabled
                //    IRQs prevent concurrent region access.
                if let Err(err) = unsafe { local.clear(paddr, &irq_guard) } {
                    error!("failed to clear remote VMCS: {:?}", err);
                }
            }
        })
        .wait();

        // No other caller can reload this VMCS before we return. A failed
        // VMCLEAR retains both the region and its residency for a later retry.
        if region.active_cpu.lock().is_some() {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    /// Clears an active VMCS on the current CPU.
    ///
    /// # Safety
    ///
    /// 1. `self` must be the current CPU's VMCS state, and this CPU must remain
    ///    in VMX root operation throughout the call.
    /// 2. `paddr` must identify a valid, page-aligned, initialized VMCS region.
    unsafe fn clear(&mut self, paddr: Paddr, irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        let region = self.active.get(&paddr).ok_or(Error::InvalidArgs)?;
        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmclear(paddr, irq_guard) }?;

        *region.active_cpu.lock() = None;
        self.active.remove(&paddr);
        if self.current == Some(paddr) {
            self.current = None;
        }
        Ok(())
    }

    /// Clears every active VMCS on this CPU before VMX shutdown.
    pub(super) fn deactivate_all(irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        let local = LOCAL_VMCS_STATE.get_with(irq_guard);
        let mut local = local.lock();
        while let Some((&paddr, _)) = local.active.first_key_value() {
            // SAFETY:
            // 1. This is the current CPU's state. The execution order ensures
            //    that the current CPU is in VMX operation at this point.
            // 2. This CPU's active set retains the valid VMCS region.
            unsafe { local.clear(paddr, irq_guard) }?;
        }
        Ok(())
    }
}

#[cfg(ktest)]
mod test {
    use x86::vmx::vmcs::guest::RIP;

    use super::*;
    use crate::cpu;

    #[ktest]
    fn switch_and_reuse_vmcs() {
        let vmx = VmxGuard::acquire_vmx().expect("VMX is required");
        let mut first = Vmcs::new(&vmx).unwrap();
        let mut second = Vmcs::new(&vmx).unwrap();
        let _preempt_guard = task::disable_preempt();

        first.load(&vmx).unwrap();
        first.write(RIP, 0x1000).unwrap();
        second.load(&vmx).unwrap();
        second.write(RIP, 0x2000).unwrap();
        first.load(&vmx).unwrap();
        assert_eq!(first.read(RIP).unwrap(), 0x1000);

        // VMX shutdown clears active VMCSs, which can be loaded again later.
        drop(vmx);
        assert!(first.region.active_cpu.lock().is_none());
        assert!(second.region.active_cpu.lock().is_none());
        let vmx = VmxGuard::acquire_vmx().unwrap();
        first.load(&vmx).unwrap();
        assert_eq!(first.read(RIP).unwrap(), 0x1000);
        second.load(&vmx).unwrap();
        assert_eq!(second.read(RIP).unwrap(), 0x2000);

        // Dropping a non-current VMCS preserves the current one.
        drop(first);
        assert_eq!(second.read(RIP).unwrap(), 0x2000);
        drop(second);
        // SAFETY: `vmx` keeps this CPU in VMX operation.
        assert_eq!(
            unsafe { instructions::vmptrst(&irq::disable_local()) },
            u64::MAX
        );
    }

    #[ktest]
    fn migrate_and_destroy_remote_vmcs() {
        // The test task runs on the BSP; use an IPI to load a VMCS on an AP.
        static REMOTE_VMCS: SpinLock<Option<(Vmcs, VmxGuard)>, LocalIrqDisabled> =
            SpinLock::new(None);
        let load_remote_vmcs = || {
            let mut slot = REMOTE_VMCS.lock();
            let (vmcs, vmx) = slot.as_mut().unwrap();
            vmcs.load(vmx).unwrap();
            vmcs.write(RIP, 0x3000).unwrap();
        };

        let vmx = VmxGuard::acquire_vmx().expect("VMX is required");
        let vmcs = Vmcs::new(&vmx).unwrap();
        let preempt_guard = task::disable_preempt();
        let remote_cpu = cpu::all_cpus()
            .find(|cpu| *cpu != preempt_guard.current_cpu())
            .expect("two CPUs are required");
        let targets = CpuSet::from(remote_cpu);

        *REMOTE_VMCS.lock() = Some((vmcs, vmx));
        crate::smp::inter_processor_call(&targets, load_remote_vmcs).wait();
        let (mut vmcs, vmx) = REMOTE_VMCS.lock().take().unwrap();
        assert_eq!(*vmcs.region.active_cpu.lock(), Some(remote_cpu));

        // Migration preserves fields and clears the previous CPU's current VMCS.
        vmcs.load(&vmx).unwrap();
        assert_eq!(vmcs.read(RIP).unwrap(), 0x3000);
        assert!(
            LOCAL_VMCS_STATE
                .get_on_cpu(remote_cpu)
                .lock()
                .current
                .is_none()
        );
        drop(vmcs);

        // Dropping a remotely active VMCS clears it and releases its region.
        let vmcs = Vmcs::new(&vmx).unwrap();
        let region = Arc::downgrade(&vmcs.region);
        *REMOTE_VMCS.lock() = Some((vmcs, vmx));
        crate::smp::inter_processor_call(&targets, load_remote_vmcs).wait();
        let (vmcs, _vmx) = REMOTE_VMCS.lock().take().unwrap();
        drop(vmcs);
        assert!(region.upgrade().is_none());
    }
}
