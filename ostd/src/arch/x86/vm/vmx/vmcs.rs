// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use x86::msr::{IA32_VMX_BASIC, rdmsr};

use super::{VMX_GUARD_STATE, VmxGuard, context_switch, instructions};
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

/// A VMCS region and its activation state.
///
/// Callers should disable preemption across loading and field access.
/// Active VMCSs are retained by their CPU until cleared.
pub(in crate::arch::vm) struct Vmcs {
    frame: Frame<VmcsRegionMeta>,
    state: SpinLock<VmcsState, LocalIrqDisabled>,
}

struct VmcsState {
    is_initialized: bool,
    active_cpu: Option<CpuId>,
    is_launched: bool,
}

impl Vmcs {
    /// Allocates an inactive VMCS.
    pub fn new() -> Result<Self> {
        let frame = FrameAllocOptions::new().alloc_frame_with(VmcsRegionMeta)?;

        Ok(Self {
            frame,
            state: SpinLock::new(VmcsState {
                is_initialized: false,
                active_cpu: None,
                is_launched: false,
            }),
        })
    }

    /// Makes this VMCS current, clearing it on its previous CPU if necessary.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    pub fn load(self: &Arc<Self>, vmx_guard: &VmxGuard) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        LocalVmcsState::activate(self, vmx_guard)
    }

    /// Clears this VMCS on its active CPU.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled if the VMCS is active.
    pub fn deactivate(&self) -> Result<()> {
        if self.state.lock().active_cpu.is_none() {
            return Ok(());
        }

        assert!(crate::arch::irq::is_local_enabled());
        let _lifecycle = VMX_GUARD_STATE.lock();
        LocalVmcsState::deactivate(self)
    }

    /// Reads a field, returning an error if this VMCS is not current on this CPU.
    pub fn read(&self, field: u32) -> Result<usize> {
        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let local = local.lock();
        if local.current != Some(self.frame.paddr()) {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The current entry identifies this retained, active VMCS, so
        // VMX is enabled. The local lock and IRQ guard exclude switching,
        // clearing and shutdown during this read.
        unsafe { instructions::vmread(field, &irq_guard) }
    }

    /// Writes a field, returning an error if this VMCS is not current on this CPU.
    pub fn write(&self, field: u32, value: usize) -> Result<()> {
        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let local = local.lock();
        if local.current != Some(self.frame.paddr()) {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The current entry, local lock and IRQ guard keep this VMCS
        // current and VMX enabled as in `read`, and serialize field access.
        unsafe { instructions::vmwrite(field, value, &irq_guard) }
    }

    /// Enters the guest using this VMCS's hardware launch state.
    ///
    /// # Safety
    ///
    /// 1. The VMCS must satisfy the guest isolation and host-state requirements
    ///    of [`context_switch::vcpu_run`].
    /// 2. Resources referenced by the VMCS must remain alive through the VM exit.
    /// 3. The caller must save and restore state not managed by VMX before
    ///    allowing interrupts or scheduling on this CPU.
    pub unsafe fn run(
        &self,
        regs: &mut crate::arch::vm::VcpuRegs,
        irq_guard: &DisabledLocalIrqGuard,
    ) -> Result<()> {
        let is_launched = self.state.lock().is_launched;
        // SAFETY: The caller ensures safety.
        let result = unsafe { context_switch::vcpu_run(regs, is_launched, irq_guard) };
        if result != 0 {
            return Err(Error::InvalidArgs);
        }
        self.state.lock().is_launched = true;
        Ok(())
    }
}

struct VmcsRegionMeta;
crate::impl_frame_meta_for!(VmcsRegionMeta);

pub(super) struct LocalVmcsState {
    current: Option<Paddr>,
    active: BTreeMap<Paddr, Arc<Vmcs>>,
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

    fn activate(vmcs: &Arc<Vmcs>, _vmx_guard: &VmxGuard) -> Result<()> {
        let preempt_guard = task::disable_preempt();
        let cpu = preempt_guard.current_cpu();
        let active_cpu = vmcs.state.lock().active_cpu;
        if active_cpu.is_some_and(|active_cpu| active_cpu != cpu) {
            Self::deactivate(vmcs)?;
        }

        let irq_guard = irq::disable_local();
        let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
        let mut local = local.lock();
        let paddr = vmcs.frame.paddr();
        if local.current == Some(paddr) {
            return Ok(());
        }

        let was_active = local.active.contains_key(&paddr);
        if !was_active {
            if active_cpu.is_none() {
                let mut state = vmcs.state.lock();
                if !state.is_initialized {
                    // SAFETY: `_vmx_guard` keeps this CPU in VMX operation.
                    let basic = unsafe { rdmsr(IA32_VMX_BASIC) };
                    let revision_id = basic as u32 & 0x7fff_ffff;
                    // SAFETY: The typed frame has never been activated, and
                    // the state lock serializes initialization.
                    unsafe { (paddr_to_vaddr(paddr) as *mut u32).write(revision_id) };
                    state.is_initialized = true;
                }

                // SAFETY:
                // 1. `_vmx_guard` keeps this CPU in VMX root operation.
                // 2. The typed frame is initialized and inactive on every CPU.
                unsafe { instructions::vmclear(paddr, &irq_guard) }?;
            }
            local.active.insert(paddr, vmcs.clone());
        }

        // SAFETY:
        // 1. `_vmx_guard` keeps this CPU in VMX root operation.
        // 2. The region is initialized and cleared on any previous CPU.
        if let Err(err) = unsafe { instructions::vmptrld(paddr, &irq_guard) } {
            if !was_active {
                local.active.remove(&paddr);
            }
            return Err(err);
        }
        local.current = Some(paddr);
        vmcs.state.lock().active_cpu = Some(cpu);
        Ok(())
    }

    fn deactivate(vmcs: &Vmcs) -> Result<()> {
        let active_cpu = vmcs.state.lock().active_cpu;
        let Some(cpu) = active_cpu else {
            return Ok(());
        };
        let preempt_guard = task::disable_preempt();
        let paddr = vmcs.frame.paddr();
        if cpu == preempt_guard.current_cpu() {
            let irq_guard = irq::disable_local();
            // SAFETY:
            // 1. This is the current CPU's state. The caller's VMX guard or
            //    lifecycle lock excludes shutdown.
            // 2. `paddr` comes from the typed VMCS region active on this CPU;
            //    the owning context is exclusively borrowed, and disabled
            //    IRQs prevent concurrent field access.
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

        crate::smp::inter_processor_call(&CpuSet::from(cpu), || {
            let irq_guard = irq::disable_local();
            let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
            let mut local = local.lock();
            while let Some(paddr) = local.pending_clear.pop() {
                // SAFETY:
                // 1. This is the current CPU's state. The caller keeps its VMX
                //    guard or lifecycle lock until this IPI completes.
                // 2. Each queued address identifies a retained VMCS region active
                //    on this CPU. The callers exclusively borrow their owning
                //    contexts; the local lock and disabled IRQs exclude field access.
                if let Err(err) = unsafe { local.clear(paddr, &irq_guard) } {
                    error!("failed to clear remote VMCS: {:?}", err);
                }
            }
        })
        .wait();

        if vmcs.state.lock().active_cpu.is_some() {
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
        let vmcs = self.active.get(&paddr).ok_or(Error::InvalidArgs)?;
        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmclear(paddr, irq_guard) }?;

        let mut state = vmcs.state.lock();
        state.is_launched = false;
        state.active_cpu = None;
        drop(state);
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
    use crate::arch::cpu::extension::{IsaExtensions, has_extensions};

    #[ktest]
    fn switch_and_reuse_vmcs() {
        // Passes if VMX is not supported.
        if !has_extensions(IsaExtensions::VMX) {
            crate::early_print!(" [skipped: VMX unavailable]");
            return;
        }

        let vmx = VmxGuard::acquire_vmx().expect("VMX is required");
        let first = Arc::new(Vmcs::new().unwrap());
        let second = Arc::new(Vmcs::new().unwrap());
        let _preempt_guard = task::disable_preempt();

        first.load(&vmx).unwrap();
        first.write(RIP, 0x1000).unwrap();
        second.load(&vmx).unwrap();
        second.write(RIP, 0x2000).unwrap();
        first.load(&vmx).unwrap();
        assert_eq!(first.read(RIP).unwrap(), 0x1000);

        // VMX shutdown clears active VMCSs, which can be loaded again later.
        drop(vmx);
        assert!(first.state.lock().active_cpu.is_none());
        assert!(second.state.lock().active_cpu.is_none());
        let vmx = VmxGuard::acquire_vmx().unwrap();
        first.load(&vmx).unwrap();
        assert_eq!(first.read(RIP).unwrap(), 0x1000);
        second.load(&vmx).unwrap();
        assert_eq!(second.read(RIP).unwrap(), 0x2000);

        // Clearing a non-current VMCS preserves the current one.
        first.deactivate().unwrap();
        drop(first);
        assert_eq!(second.read(RIP).unwrap(), 0x2000);
        second.deactivate().unwrap();
        drop(second);
        // SAFETY: `vmx` keeps this CPU in VMX operation.
        assert_eq!(
            unsafe { instructions::vmptrst(&irq::disable_local()) },
            u64::MAX
        );
    }
}
