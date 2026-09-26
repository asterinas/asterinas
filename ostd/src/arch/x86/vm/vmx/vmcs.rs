// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use x86::msr::{IA32_VMX_BASIC, rdmsr};

use super::{VmxGuard, instructions};
use crate::{
    Error,
    arch::cpu::extension::{IsaExtensions, has_extensions},
    cpu::{CpuId, CpuSet, PinCurrentCpu},
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    mm::{Frame, FrameAllocOptions, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, SpinLock},
    task,
};

/// A reference-counted VMCS.
///
/// Callers should disable preemption across loading and field access.
pub(in crate::arch::vm) type Vmcs = Frame<VmcsRegionMeta>;

pub(in crate::arch::vm) struct VmcsRegionMeta {
    state: SpinLock<VmcsState>,
}
crate::impl_frame_meta_for!(VmcsRegionMeta);

struct VmcsState {
    active_cpu: Option<CpuId>,
}

impl Vmcs {
    /// Allocates an inactive VMCS.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) fn new() -> Result<Self> {
        if !has_extensions(IsaExtensions::VMX) {
            return Err(Error::NotEnoughResources);
        }

        // SAFETY: A CPU that enumerates VMX provides the IA32_VMX_BASIC MSR.
        let basic = unsafe { rdmsr(IA32_VMX_BASIC) };
        // OSTD's linear mapping uses write-back memory. See Intel SDM,
        // Vol. 3D, Appendix A.1, for the VMCS memory-type requirement.
        const VMCS_MEMORY_TYPE_WB: u64 = 6;
        if (basic >> 50) & 0xf != VMCS_MEMORY_TYPE_WB {
            return Err(Error::NotEnoughResources);
        }

        let frame = FrameAllocOptions::new().alloc_frame_with(VmcsRegionMeta {
            state: SpinLock::new(VmcsState { active_cpu: None }),
        })?;
        let revision_id = basic as u32 & 0x7fff_ffff;
        // SAFETY: The frame is exclusively owned and has never been activated.
        unsafe { (paddr_to_vaddr(frame.paddr()) as *mut u32).write(revision_id) };

        Ok(frame)
    }

    /// Makes this VMCS current, clearing it on its previous CPU if necessary.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) fn load(&self, vmx_guard: &VmxGuard) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        // SAFETY: `vmcs_state` is the locked state of `vmcs`.
        unsafe { LocalVmcsState::do_activate(self, &mut self.meta().state.lock(), vmx_guard) }
    }

    /// Clears this VMCS on its active CPU.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) fn deactivate(&self, vmx_guard: &VmxGuard) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        // SAFETY: `vmcs_state` is the locked state of `vmcs`.
        unsafe { LocalVmcsState::do_deactivate(self, &mut self.meta().state.lock(), vmx_guard) }
    }

    /// # Safety
    ///
    /// The VMCS must be current on this CPU.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) unsafe fn read(
        &self,
        field: u32,
        irq_guard: &DisabledLocalIrqGuard,
    ) -> Result<usize> {
        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmread(field, irq_guard) }
    }

    /// # Safety
    ///
    /// 1. The VMCS must be current on this CPU.
    /// 2. The `field` and `value` must not violate the host kernel's state.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) unsafe fn write(
        &self,
        field: u32,
        value: usize,
        irq_guard: &DisabledLocalIrqGuard,
    ) -> Result<()> {
        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmwrite(field, value, irq_guard) }
    }
}

pub(super) struct LocalVmcsState {
    current: Option<Paddr>,
    active: BTreeMap<Paddr, Vmcs>,
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

    /// # Safety
    ///
    /// `vmcs_state` must be the locked state of `vmcs`.
    unsafe fn do_activate(
        vmcs: &Vmcs,
        vmcs_state: &mut VmcsState,
        _vmx_guard: &VmxGuard,
    ) -> Result<()> {
        let preempt_guard = task::disable_preempt();
        let previous_cpu = vmcs_state.active_cpu;
        if let Some(active_cpu) = previous_cpu
            && active_cpu != preempt_guard.current_cpu()
        {
            // SAFETY: `vmcs_state` is the locked state of `vmcs`.
            unsafe { Self::do_deactivate(vmcs, vmcs_state, _vmx_guard)? };
        }

        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let mut local = local.lock();
        if local.current == Some(vmcs.paddr()) {
            return Ok(());
        }

        let was_active = local.active.contains_key(&vmcs.paddr());
        if !was_active {
            if previous_cpu.is_none() {
                // SAFETY:
                // 1. `_vmx_guard` keeps this CPU in VMX root operation.
                // 2. The VMCS region is initialized. `previous_cpu` is `None` means
                //    this VMCS has not been active on any CPU before.
                unsafe { instructions::vmclear(vmcs.paddr(), &irq_guard) }?;
            }
            local.active.insert(vmcs.paddr(), vmcs.clone());
        }

        // SAFETY:
        // 1. `_vmx_guard` keeps this CPU in VMX root operation.
        // 2. The region is initialized and cleared on any previous CPU.
        if let Err(err) = unsafe { instructions::vmptrld(vmcs.paddr(), &irq_guard) } {
            if !was_active {
                local.active.remove(&vmcs.paddr());
            }
            return Err(err);
        }
        local.current = Some(vmcs.paddr());
        vmcs_state.active_cpu = Some(preempt_guard.current_cpu());
        Ok(())
    }

    /// # Safety
    ///
    /// `vmcs_state` must be the locked state of `vmcs`.
    unsafe fn do_deactivate(
        vmcs: &Vmcs,
        vmcs_state: &mut VmcsState,
        _vmx_guard: &VmxGuard,
    ) -> Result<()> {
        let Some(cpu) = vmcs_state.active_cpu else {
            return Ok(());
        };
        let preempt_guard = task::disable_preempt();
        if cpu == preempt_guard.current_cpu() {
            let irq_guard = irq::disable_local();
            // SAFETY:
            // 1. This is the current CPU's state.
            // 2  `_vmx_guard` keeps all CPUs in VMX root operation.
            // 3. `vmcs.paddr()` is the physical address of the active VMCS region
            //    on this CPU.
            unsafe {
                LOCAL_VMCS_STATE
                    .get_with(&irq_guard)
                    .lock()
                    .clear(vmcs.paddr(), &irq_guard)?
            };
            vmcs_state.active_cpu = None;
            return Ok(());
        }

        assert!(crate::arch::irq::is_local_enabled());
        LOCAL_VMCS_STATE
            .get_on_cpu(cpu)
            .lock()
            .pending_clear
            .push(vmcs.paddr());

        crate::smp::inter_processor_call(&CpuSet::from(cpu), || {
            let irq_guard = irq::disable_local();
            let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
            let mut local = local.lock();
            while let Some(paddr) = local.pending_clear.pop() {
                // SAFETY:
                // 1. This is the current CPU's state.
                // 2. `_vmx_guard` keeps all CPUs in VMX root operation.
                // 3. Each queued address identifies a retained VMCS region active
                //    on this CPU.
                if let Err(err) = unsafe { local.clear(paddr, &irq_guard) } {
                    panic!("failed to clear remote VMCS: {:?}", err);
                }
            }
        })
        .wait();
        vmcs_state.active_cpu = None;
        Ok(())
    }

    /// Clears an active VMCS on the current CPU.
    ///
    /// # Safety
    ///
    /// 1. `self` must be the current CPU's VMCS state
    /// 2. This CPU must remain in VMX root operation throughout the call.
    /// 3. `paddr` must identify a valid, page-aligned, initialized VMCS region
    ///    which is currently active on this CPU.
    unsafe fn clear(&mut self, paddr: Paddr, irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmclear(paddr, irq_guard) }?;

        self.active.remove(&paddr);
        if self.current == Some(paddr) {
            self.current = None;
        }
        Ok(())
    }

    /// Clears every active VMCS on this CPU before VMX shutdown.
    ///
    /// # Safety
    ///
    /// 1. This CPU must remain in VMX root operation throughout the call.
    /// 2. There is no concurrent VMCS activation and deactivation.
    pub(super) unsafe fn deactivate_all(irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        let local = LOCAL_VMCS_STATE.get_with(irq_guard);
        // The caller excludes VMCS activation and deactivation while `local` is unlocked.
        let active = {
            let mut local = local.lock();
            local.current = None;
            core::mem::take(&mut local.active)
        };
        for (paddr, vmcs) in active {
            // SAFETY:
            // 1. The caller ensures this CPU remains in VMX root operation.
            // 2. Each active VMCS region is valid, page-aligned, and initialized,
            //    which is currently active on this CPU.
            unsafe { instructions::vmclear(paddr, irq_guard) }?;
            vmcs.meta().state.lock().active_cpu = None;
        }
        Ok(())
    }
}

#[cfg(ktest)]
mod test {
    use x86::vmx::vmcs::guest::RIP;

    use super::*;

    #[ktest]
    fn switch_and_reuse_vmcs() {
        // Passes if VMX is not supported.
        if !has_extensions(IsaExtensions::VMX) {
            crate::early_print!(" [skipped: VMX unavailable]");
            return;
        }

        let first = Vmcs::new().unwrap();
        let second = Vmcs::new().unwrap();
        let vmx = VmxGuard::acquire_vmx().expect("VMX is required");
        let _preempt_guard = task::disable_preempt();

        // Fields from the current VMCS can be read and write.
        first.load(&vmx).unwrap();
        unsafe { first.write(RIP, 0x1000, &irq::disable_local()).unwrap() };
        second.load(&vmx).unwrap();
        unsafe { second.write(RIP, 0x2000, &irq::disable_local()).unwrap() };
        first.load(&vmx).unwrap();
        assert_eq!(
            unsafe { first.read(RIP, &irq::disable_local()).unwrap() },
            0x1000
        );

        // VMX shutdown clears active VMCSs, which can be loaded again later.
        drop(vmx);
        assert!(first.meta().state.lock().active_cpu.is_none());
        assert!(second.meta().state.lock().active_cpu.is_none());
        let vmx = VmxGuard::acquire_vmx().unwrap();
        first.load(&vmx).unwrap();
        assert_eq!(
            unsafe { first.read(RIP, &irq::disable_local()).unwrap() },
            0x1000
        );
        second.load(&vmx).unwrap();
        assert_eq!(
            unsafe { second.read(RIP, &irq::disable_local()).unwrap() },
            0x2000
        );

        // Clearing a non-current VMCS preserves the current one.
        first.deactivate(&vmx).unwrap();
        drop(first);
        assert_eq!(
            unsafe { second.read(RIP, &irq::disable_local()).unwrap() },
            0x2000
        );
        second.deactivate(&vmx).unwrap();
        drop(second);
    }
}
