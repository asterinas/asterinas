// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;
use core::cell::{RefCell, RefMut};

use x86::msr::{IA32_VMX_BASIC, rdmsr};

use super::{VmxGuard, context_switch, instructions};
use crate::{
    Error,
    cpu::{CpuId, CpuSet, PinCurrentCpu},
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    mm::{Frame, FrameAllocOptions, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, SpinLock},
    task::{self, atomic_mode::AsAtomicModeGuard},
};

/// A reference-counted VMCS.
pub(in crate::arch::vm) type Vmcs = Frame<VmcsRegionMeta>;

pub(in crate::arch::vm) struct VmcsRegionMeta {
    state: SpinLock<VmcsState>,
}
crate::impl_frame_meta_for!(VmcsRegionMeta);

struct VmcsState {
    active_cpu: Option<CpuId>,
    is_launched: bool,
    are_fixed_fields_initialized: bool,
}

impl Vmcs {
    /// Allocates an inactive VMCS.
    pub(in crate::arch::vm) fn new(_vmx_guard: &VmxGuard) -> Result<Self> {
        // SAFETY: `_vmx_guard` guarantees VMX support on every CPU.
        let revision_id = unsafe { rdmsr(IA32_VMX_BASIC) } as u32 & 0x7fff_ffff;
        let frame = FrameAllocOptions::new().alloc_frame_with(VmcsRegionMeta {
            state: SpinLock::new(VmcsState {
                active_cpu: None,
                is_launched: false,
                are_fixed_fields_initialized: false,
            }),
        })?;
        // SAFETY: The frame is exclusively owned and has never been activated.
        unsafe { (paddr_to_vaddr(frame.paddr()) as *mut u32).write(revision_id) };

        Ok(frame)
    }

    /// Makes this VMCS current and initializes its fixed fields if needed.
    ///
    /// Returns exclusive access to the current VMCS. Local IRQs remain disabled
    /// until `irq_guard` is dropped.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled and `irq_guard` must be `None`.
    pub(in crate::arch::vm) fn load<'a>(
        &'a self,
        vmx_guard: &'a VmxGuard,
        irq_guard: &'a mut Option<DisabledLocalIrqGuard>,
    ) -> Result<CurrentVmcs<'a>> {
        assert!(crate::arch::irq::is_local_enabled());
        assert!(irq_guard.is_none());
        let mut state = self.meta().state.lock();
        if state.active_cpu != Some(state.as_atomic_mode_guard().current_cpu()) {
            // Controls and host state depend on the active CPU.
            state.are_fixed_fields_initialized = false;
        }
        // SAFETY: `state` is the locked state of this VMCS.
        let current =
            unsafe { LocalVmcsState::do_activate(self, &mut state, vmx_guard, irq_guard) }?;

        if !state.are_fixed_fields_initialized {
            current.setup_fixed_controls(vmx_guard)?;
            current.setup_fixed_host()?;
            state.are_fixed_fields_initialized = true;
        }

        Ok(current)
    }

    /// Clears this VMCS on its active CPU.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    pub(in crate::arch::vm) fn deactivate(&self, vmx_guard: &VmxGuard) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        // SAFETY: `vmcs_state` is the locked state of `vmcs`.
        unsafe { LocalVmcsState::do_deactivate(self, &mut self.meta().state.lock(), vmx_guard) }
    }
}

/// Exclusive access to the current CPU's VMCS with local IRQs disabled.
pub(in crate::arch::vm) struct CurrentVmcs<'a> {
    vmcs: &'a Vmcs,
    irq_guard: &'a DisabledLocalIrqGuard,
    _local_guard: RefMut<'a, LocalVmcsState>,
    _vmx_guard: &'a VmxGuard,
}

impl CurrentVmcs<'_> {
    /// Returns the guard keeping local IRQs disabled.
    pub(in crate::arch::vm) fn irq_guard(&self) -> &DisabledLocalIrqGuard {
        self.irq_guard
    }

    /// Reads a field from this VMCS.
    pub(in crate::arch::vm) fn read(&self, field: u32) -> Result<usize> {
        // SAFETY: The guards keep VMX enabled and this VMCS current on this CPU.
        unsafe { instructions::vmread(field, self.irq_guard) }
    }

    /// Writes a field in this VMCS.
    ///
    /// # Safety
    ///
    /// The `field` and `value` must not violate the host kernel's state.
    pub(in crate::arch::vm) unsafe fn write(&self, field: u32, value: usize) -> Result<()> {
        // SAFETY:
        // 1. The guards keep VMX enabled and this VMCS current on this CPU.
        // 2. The caller ensures the `field` and `value` preserve the host kernel's state.
        unsafe { instructions::vmwrite(field, value, self.irq_guard) }
    }

    /// Enters the guest using this VMCS's hardware launch state.
    ///
    /// # Safety
    ///
    /// 1. The VMCS must satisfy the guest isolation and host-state requirements
    ///    of [`context_switch::vcpu_run`].
    /// 2. Resources referenced by the VMCS must remain alive through the VM exit.
    /// 3. The caller must restore host MSRs not managed by VMX before allowing
    ///    interrupts or scheduling on this CPU, including if guest entry fails.
    pub(in crate::arch::vm) unsafe fn run(
        &self,
        regs: &mut crate::arch::vm::VcpuRegs,
    ) -> Result<()> {
        let is_launched = self.vmcs.meta().state.lock().is_launched;
        // SAFETY:
        // 1. The guards retain exclusive access to this current VMCS in VMX operation.
        //    Its launch state is updated after each successful entry and clear.
        // 2. The caller ensures guest isolation, host state and resource lifetimes.
        // 3. The caller ensures host MSRs are restored before enabling IRQs.
        let result = unsafe { context_switch::vcpu_run(regs, is_launched, self.irq_guard) };
        if result != 0 {
            return Err(Error::InvalidArgs);
        }
        self.vmcs.meta().state.lock().is_launched = true;
        Ok(())
    }
}

pub(super) struct LocalVmcsState {
    current: Option<Paddr>,
    active: BTreeMap<Paddr, Vmcs>,
}

cpu_local! {
    static LOCAL_VMCS_STATE: RefCell<LocalVmcsState> =
        RefCell::new(LocalVmcsState::new());
    /// The VMCS regions queued for clearing on this CPU.
    static PENDING_CLEAR: SpinLock<Vec<Paddr>, LocalIrqDisabled> =
        SpinLock::new(Vec::new());
}

impl LocalVmcsState {
    const fn new() -> Self {
        Self {
            current: None,
            active: BTreeMap::new(),
        }
    }

    /// # Safety
    ///
    /// `vmcs_state` must be the locked state of `vmcs`.
    unsafe fn do_activate<'a>(
        vmcs: &'a Vmcs,
        vmcs_state: &mut VmcsState,
        vmx_guard: &'a VmxGuard,
        irq_guard: &'a mut Option<DisabledLocalIrqGuard>,
    ) -> Result<CurrentVmcs<'a>> {
        let preempt_guard = task::disable_preempt();
        let previous_cpu = vmcs_state.active_cpu;
        if let Some(active_cpu) = previous_cpu
            && active_cpu != preempt_guard.current_cpu()
        {
            // SAFETY: `vmcs_state` is the locked state of `vmcs`.
            unsafe { Self::do_deactivate(vmcs, vmcs_state, vmx_guard)? };
        }

        let irq_guard = irq_guard.insert(irq::disable_local());
        let local = LOCAL_VMCS_STATE.get_with(irq_guard);
        // SAFETY: The CPU-local storage outlives `'a`, and `irq_guard` pins this
        // CPU throughout the returned borrow.
        let local: &'a RefCell<Self> = unsafe { &*core::ptr::from_ref(&*local) };
        let mut local = local.borrow_mut();
        if local.current == Some(vmcs.paddr()) {
            return Ok(CurrentVmcs {
                vmcs,
                irq_guard,
                _local_guard: local,
                _vmx_guard: vmx_guard,
            });
        }

        let was_active = local.active.contains_key(&vmcs.paddr());
        if !was_active {
            if previous_cpu.is_none() {
                // SAFETY:
                // 1. `vmx_guard` keeps this CPU in VMX root operation.
                // 2. The VMCS region is initialized. `previous_cpu` is `None` means
                //    this VMCS has not been active on any CPU before.
                unsafe { instructions::vmclear(vmcs.paddr(), irq_guard) }?;
                vmcs_state.is_launched = false;
            }
            local.active.insert(vmcs.paddr(), vmcs.clone());
        }

        // SAFETY:
        // 1. `vmx_guard` keeps this CPU in VMX root operation.
        // 2. The region is initialized and cleared on any previous CPU.
        if let Err(err) = unsafe { instructions::vmptrld(vmcs.paddr(), irq_guard) } {
            if !was_active {
                local.active.remove(&vmcs.paddr());
            }
            return Err(err);
        }
        local.current = Some(vmcs.paddr());
        vmcs_state.active_cpu = Some(preempt_guard.current_cpu());
        Ok(CurrentVmcs {
            vmcs,
            irq_guard,
            _local_guard: local,
            _vmx_guard: vmx_guard,
        })
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
                    .borrow_mut()
                    .clear(vmcs.paddr(), &irq_guard)?
            };
            vmcs_state.active_cpu = None;
            vmcs_state.is_launched = false;
            return Ok(());
        }

        assert!(crate::arch::irq::is_local_enabled());
        PENDING_CLEAR.get_on_cpu(cpu).lock().push(vmcs.paddr());

        crate::smp::inter_processor_call(&CpuSet::from(cpu), || {
            let irq_guard = irq::disable_local();
            let pending_clear = PENDING_CLEAR.get_with(&irq_guard);
            let mut pending_clear = pending_clear.lock();
            let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
            let mut local = local.borrow_mut();
            while let Some(paddr) = pending_clear.pop() {
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
        vmcs_state.is_launched = false;
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
        // Deactivating without holding the VMCS's `state` lock is safe because
        // the caller excludes concurrent VMCS activation and deactivation.
        let active = {
            let mut local = local.borrow_mut();
            local.current = None;
            core::mem::take(&mut local.active)
        };
        for (paddr, vmcs) in active {
            // SAFETY:
            // 1. The caller ensures this CPU remains in VMX root operation.
            // 2. Each active VMCS region is valid, page-aligned, and initialized,
            //    which is currently active on this CPU.
            unsafe { instructions::vmclear(paddr, irq_guard) }?;
            let mut state = vmcs.meta().state.lock();
            state.active_cpu = None;
            state.is_launched = false;
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
        let first = Vmcs::new(&vmx).unwrap();
        let second = Vmcs::new(&vmx).unwrap();

        // Fields from the current VMCS can be read and written.
        {
            let mut irq_guard = None;
            let current = first.load(&vmx, &mut irq_guard).unwrap();
            // SAFETY: The guest instruction pointer does not affect host state.
            unsafe { current.write(RIP, 0x1000).unwrap() };
        }
        {
            let mut irq_guard = None;
            let current = second.load(&vmx, &mut irq_guard).unwrap();
            // SAFETY: The guest instruction pointer does not affect host state.
            unsafe { current.write(RIP, 0x2000).unwrap() };
        }
        {
            let mut irq_guard = None;
            let current = first.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x1000);
        }

        // VMX shutdown clears active VMCSs, which can be loaded again later.
        drop(vmx);
        assert!(first.meta().state.lock().active_cpu.is_none());
        assert!(second.meta().state.lock().active_cpu.is_none());
        let vmx = VmxGuard::acquire_vmx().unwrap();
        let preempt_guard = task::disable_preempt();
        {
            let mut irq_guard = None;
            let current = first.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x1000);
        }
        {
            let mut irq_guard = None;
            let current = second.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x2000);
        }

        // Clearing a non-current VMCS preserves the current one.
        first.deactivate(&vmx).unwrap();
        drop(first);
        {
            let irq_guard = irq::disable_local();
            // SAFETY: `vmx` keeps this CPU in VMX root operation.
            assert_eq!(
                unsafe { instructions::vmptrst(&irq_guard) },
                second.paddr() as u64
            );
        }
        {
            let mut irq_guard = None;
            let current = second.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x2000);
        }
        second.deactivate(&vmx).unwrap();
        drop(second);
        drop(preempt_guard);
    }
}
