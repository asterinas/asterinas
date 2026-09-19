// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use x86::msr::{IA32_VMX_BASIC, rdmsr};

use super::{VmxGuard, instructions};
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
pub(in crate::arch::vm) struct Vmcs {
    frame: Frame<VmcsRegionMeta>,
    state: SpinLock<VmcsState, LocalIrqDisabled>,
}

struct VmcsState {
    is_initialized: bool,
    active_cpu: Option<CpuId>,
}

impl Vmcs {
    /// Allocates an inactive VMCS.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn new() -> Result<Self> {
        let frame = FrameAllocOptions::new().alloc_frame_with(VmcsRegionMeta)?;

        Ok(Self {
            frame,
            state: SpinLock::new(VmcsState {
                is_initialized: false,
                active_cpu: None,
            }),
        })
    }

    /// Makes this VMCS current, clearing it on its previous CPU if necessary.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) fn load(self: &Arc<Self>, vmx_guard: &VmxGuard) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        LocalVmcsState::activate(self, vmx_guard)
    }

    /// Clears this VMCS on its active CPU.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::arch::vm) fn deactivate(&self) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        LocalVmcsState::deactivate(self)
    }

    /// # Safety
    ///
    /// 1. The VMCS must be current on this CPU.
    /// 2. Local IRQs must be disabled to prevent preemption and VMCS switching.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub unsafe fn read(&self, field: u32) -> Result<usize> {
        let irq_guard = irq::disable_local();

        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmread(field, &irq_guard) }
    }

    /// # Safety
    ///
    /// 1. The VMCS must be current on this CPU.
    /// 2. Local IRQs must be disabled to prevent preemption and VMCS switching.
    /// 3. The `field` and `value` must not violate the host kernel's state.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub unsafe fn write(&self, field: u32, value: usize) -> Result<()> {
        let irq_guard = irq::disable_local();

        // SAFETY: The caller ensures safety.
        unsafe { instructions::vmwrite(field, value, &irq_guard) }
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
                    // OSTD's linear mapping uses write-back memory. See Intel SDM,
                    // Vol. 3D, Appendix A.1, for the VMCS memory-type requirement.
                    const VMCS_MEMORY_TYPE_WB: u64 = 6;
                    if (basic >> 50) & 0xf != VMCS_MEMORY_TYPE_WB {
                        return Err(Error::NotEnoughResources);
                    }

                    let revision_id = basic as u32 & 0x7fff_ffff;
                    // SAFETY: The typed frame has never been activated, and
                    // the state lock serializes initialization.
                    unsafe { (paddr_to_vaddr(paddr) as *mut u32).write(revision_id) };
                    state.is_initialized = true;
                }

                // SAFETY:
                // 1. `_vmx_guard` keeps this CPU in VMX root operation.
                // 2. The VMCS region is initialized.
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
            // 2. `paddr` comes from the typed VMCS region active on this CPU.
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
                //    on this CPU.
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

        vmcs.state.lock().active_cpu = None;
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
        unsafe { first.write(RIP, 0x1000).unwrap() };
        second.load(&vmx).unwrap();
        unsafe { second.write(RIP, 0x2000).unwrap() };
        first.load(&vmx).unwrap();
        assert_eq!(unsafe { first.read(RIP).unwrap() }, 0x1000);

        // VMX shutdown clears active VMCSs, which can be loaded again later.
        drop(vmx);
        assert!(first.state.lock().active_cpu.is_none());
        assert!(second.state.lock().active_cpu.is_none());
        let vmx = VmxGuard::acquire_vmx().unwrap();
        first.load(&vmx).unwrap();
        assert_eq!(unsafe { first.read(RIP).unwrap() }, 0x1000);
        second.load(&vmx).unwrap();
        assert_eq!(unsafe { second.read(RIP).unwrap() }, 0x2000);

        // Clearing a non-current VMCS preserves the current one.
        first.deactivate().unwrap();
        drop(first);
        assert_eq!(unsafe { second.read(RIP).unwrap() }, 0x2000);
        second.deactivate().unwrap();
        drop(second);
        // SAFETY: `vmx` keeps this CPU in VMX operation.
        assert_eq!(
            unsafe { instructions::vmptrst(&irq::disable_local()) },
            u64::MAX
        );
    }
}
