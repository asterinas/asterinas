// SPDX-License-Identifier: MPL-2.0

//! Intel VMX platform lifecycle management.

mod instructions;

use x86::msr::{
    IA32_FEATURE_CONTROL, IA32_VMX_BASIC, IA32_VMX_CR0_FIXED0, IA32_VMX_CR0_FIXED1,
    IA32_VMX_CR4_FIXED0, IA32_VMX_CR4_FIXED1, rdmsr, wrmsr,
};
use x86_64::registers::control::{Cr0, Cr4, Cr4Flags};

use crate::{
    Error,
    arch::cpu::extension::{IsaExtensions, has_extensions},
    cpu::{CpuSet, all_cpus},
    cpu_local,
    mm::{Frame, FrameAllocOptions, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, Mutex, SpinLock},
};

const FEATURE_CONTROL_LOCKED: u64 = 1;
const FEATURE_CONTROL_VMX_OUTSIDE_SMX: u64 = 1 << 2;

pub(super) fn init_feature_control() {
    if !has_extensions(IsaExtensions::VMX) {
        return;
    }

    let feature_control = unsafe { rdmsr(IA32_FEATURE_CONTROL) };
    if feature_control & FEATURE_CONTROL_LOCKED == 0 {
        unsafe {
            wrmsr(
                IA32_FEATURE_CONTROL,
                FEATURE_CONTROL_LOCKED | FEATURE_CONTROL_VMX_OUTSIDE_SMX,
            );
        }
    }
}

struct VmxonRegionMeta;
crate::impl_frame_meta_for!(VmxonRegionMeta);

struct VmxCpuState {
    enabled: bool,
    region: Option<Frame<VmxonRegionMeta>>,
    last_error: Option<Error>,
}

impl VmxCpuState {
    const fn new() -> Self {
        Self {
            enabled: false,
            region: None,
            last_error: None,
        }
    }
}

cpu_local! {
    static VMX_CPU_STATE: SpinLock<VmxCpuState, LocalIrqDisabled> =
        SpinLock::new(VmxCpuState::new());
}

struct VmxGuardState {
    active_guards: usize,
    enabled: bool,
    poisoned: bool,
}

impl VmxGuardState {
    const fn new() -> Self {
        Self {
            active_guards: 0,
            enabled: false,
            poisoned: false,
        }
    }
}

static VMX_GUARD_STATE: Mutex<VmxGuardState> = Mutex::new(VmxGuardState::new());

/// Keeps VMX operation enabled while the guard exists.
#[cfg_attr(not(ktest), expect(dead_code))]
#[must_use]
pub(crate) struct VmxGuard {
    _private: (),
}

impl VmxGuard {
    /// Acquires a lease on the VMX platform lifecycle.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(crate) fn acquire_vmx() -> Result<VmxGuard> {
        assert!(crate::arch::irq::is_local_enabled());

        let mut state = VMX_GUARD_STATE.lock();
        state.acquire_vmx()?;

        Ok(VmxGuard { _private: () })
    }
}

impl Drop for VmxGuard {
    fn drop(&mut self) {
        assert!(crate::arch::irq::is_local_enabled());

        let mut state = VMX_GUARD_STATE.lock();
        state.drop_vmx();
    }
}

impl VmxGuardState {
    fn acquire_vmx(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::InvalidArgs);
        }
        if self.enabled {
            self.active_guards += 1;
            return Ok(());
        }

        prepare_vmxon_regions()?;
        let targets = CpuSet::new_full();
        if let Err(err) = run_on_cpus(&targets, enable_vmx) {
            let rollback_targets = enabled_cpus();
            let rollback_completed = run_on_cpus(&rollback_targets, disable_vmx).is_ok();
            cleanup_prepared_regions();
            if !rollback_completed || !enabled_cpus().is_empty() {
                self.enabled = true;
                self.poisoned = true;
            }
            return Err(err);
        }

        self.enabled = true;
        self.active_guards += 1;

        Ok(())
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    fn drop_vmx(&mut self) {
        self.active_guards -= 1;
        if self.active_guards != 0 {
            return;
        }

        let targets = enabled_cpus();

        if run_on_cpus(&targets, disable_vmx).is_err() {
            self.poisoned = true;
        } else {
            self.enabled = false;
        }
        cleanup_prepared_regions();
    }
}

impl VmxCpuState {
    /// Enables VMX on current CPU.
    fn enable_vmx(&mut self) {
        let region_paddr = self
            .region
            .as_ref()
            .expect("VMX region should be prepared")
            .paddr();

        let cr4 = Cr4::read_raw();
        debug_assert!(cr4 & Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits() == 0);
        let vmx_cr4 = cr4 | Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits();
        let revision_id = match read_and_validate_capability(vmx_cr4) {
            Ok(revision_id) => revision_id,
            Err(err) => {
                self.last_error = Some(err);
                return;
            }
        };
        // SAFETY: `region_paddr` points to a valid, page-aligned, zero-initialized VMXON region.
        unsafe { initialize_vmxon_region(region_paddr, revision_id) };

        // SAFETY: `vmx_cr4` preserves the current `CR4` value, adds only
        // `CR4.VMXE`, and has been checked against the VMX fixed-bit MSRs.
        unsafe { Cr4::write_raw(vmx_cr4) };

        // SAFETY: The capability checks, control-register update, and initialized
        // region above establish the architectural prerequisites for `VMXON`.
        if let Err(error) = unsafe { instructions::vmxon(region_paddr) } {
            // SAFETY: A failed `VMXON` leaves this CPU outside VMX operation.
            unsafe { clear_vmx_enable() };
            self.last_error = Some(error);
            return;
        }

        self.enabled = true;
        self.last_error = None;
    }

    /// Disables VMX on current CPU.
    fn disable_vmx(&mut self) {
        // SAFETY:
        // 1. The CPU state is marked as enabled only after a successful `VMXON`.
        // 2. All active VMCSs on this CPU have been cleared.
        if let Err(err) = unsafe { instructions::vmxoff() } {
            self.last_error = Some(err);
            return;
        }

        // SAFETY: A successful `VMXOFF` leaves this CPU outside VMX operation.
        unsafe { clear_vmx_enable() };

        self.enabled = false;
        self.last_error = None;
    }

    fn set_region(&mut self, region: Frame<VmxonRegionMeta>) {
        self.region = Some(region);
    }

    fn take_region(&mut self) -> Option<Frame<VmxonRegionMeta>> {
        self.region.take()
    }

    fn enabled(&self) -> bool {
        self.enabled
    }
}

fn run_on_cpus(targets: &CpuSet, handler: fn()) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }

    crate::smp::inter_processor_call(targets, handler).wait();
    for cpu in targets.iter() {
        if let Some(error) = VMX_CPU_STATE.get_on_cpu(cpu).lock().last_error {
            return Err(error);
        }
    }
    Ok(())
}

fn enable_vmx() {
    let irq_guard = crate::irq::disable_local();
    let cpu_state = VMX_CPU_STATE.get_with(&irq_guard);
    cpu_state.lock().enable_vmx();
}

fn disable_vmx() {
    let irq_guard = crate::irq::disable_local();
    let cpu_state = VMX_CPU_STATE.get_with(&irq_guard);
    cpu_state.lock().disable_vmx();
}

fn prepare_vmxon_regions() -> Result<()> {
    for cpu in all_cpus() {
        let region = match FrameAllocOptions::new().alloc_frame_with(VmxonRegionMeta) {
            Ok(region) => region,
            Err(error) => {
                cleanup_prepared_regions();
                return Err(error);
            }
        };

        let mut state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
        state.set_region(region);
    }
    Ok(())
}

fn cleanup_prepared_regions() {
    for cpu in all_cpus() {
        let region = {
            let mut state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
            if state.enabled() {
                continue;
            }
            state.take_region()
        };
        drop(region);
    }
}

fn enabled_cpus() -> CpuSet {
    let mut enabled = CpuSet::new_empty();
    for cpu in all_cpus() {
        if VMX_CPU_STATE.get_on_cpu(cpu).lock().enabled() {
            enabled.add(cpu);
        }
    }
    enabled
}

/// Clears `CR4.VMXE` without changing any other `CR4` bits.
///
/// # Safety
///
/// The current CPU must be outside VMX operation.
unsafe fn clear_vmx_enable() {
    let cr4 = Cr4::read_raw();
    // SAFETY: The caller guarantees that clearing `CR4.VMXE` is permitted.
    unsafe { Cr4::write_raw(cr4 & !Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits()) };
}

fn read_and_validate_capability(vmx_cr4: u64) -> Result<u32> {
    if !has_extensions(IsaExtensions::VMX) {
        return Err(Error::NotEnoughResources);
    }

    // SAFETY: A CPU that enumerates VMX provides the architectural VMX MSRs
    // read below. This function runs independently on each target CPU.
    let (feature_control, vmx_basic, cr0_fixed0, cr0_fixed1, cr4_fixed0, cr4_fixed1) = unsafe {
        (
            rdmsr(IA32_FEATURE_CONTROL),
            rdmsr(IA32_VMX_BASIC),
            rdmsr(IA32_VMX_CR0_FIXED0),
            rdmsr(IA32_VMX_CR0_FIXED1),
            rdmsr(IA32_VMX_CR4_FIXED0),
            rdmsr(IA32_VMX_CR4_FIXED1),
        )
    };

    if feature_control & FEATURE_CONTROL_VMX_OUTSIDE_SMX == 0 {
        return Err(Error::AccessDenied);
    }

    if !control_register_is_valid(Cr0::read_raw(), cr0_fixed0, cr0_fixed1)
        || !control_register_is_valid(vmx_cr4, cr4_fixed0, cr4_fixed1)
    {
        return Err(Error::NotEnoughResources);
    }

    Ok(vmx_basic as u32 & 0x7fff_ffff)
}

fn control_register_is_valid(value: u64, fixed0: u64, fixed1: u64) -> bool {
    value & fixed0 == fixed0 && value & !fixed1 == 0
}

/// Initializes the VMXON region with the given revision ID.
///
/// # Safety
///
/// The caller must ensure that `region_paddr` points to a valid, page-aligned,
/// zero-initialized VMXON region that is exclusively owned by the current CPU.
unsafe fn initialize_vmxon_region(region_paddr: usize, revision_id: u32) {
    let region_ptr = paddr_to_vaddr(region_paddr) as *mut u32;

    // SAFETY: `region_paddr` belongs to the live, exclusively owned frame in
    // the current CPU state. The frame is linearly mapped, page-aligned, and
    // was zero-initialized before this four-byte write.
    unsafe { region_ptr.write(revision_id) };
}

#[cfg(ktest)]
mod test {
    use super::*;

    #[ktest]
    fn multiple_vmx_guards_lifecycle() {
        let guard1 = VmxGuard::acquire_vmx().expect("Failed to acquire VMX guard");
        // SAFETY: `guard1` keeps every CPU in VMX root operation, and no VMCS
        // has been made current.
        assert_eq!(unsafe { instructions::vmptrst() }, u64::MAX);

        let guard2 = VmxGuard::acquire_vmx().expect("Failed to acquire VMX guard");
        // SAFETY: `guard2` still keeps every CPU in VMX root operation, and no
        // VMCS has been made current.
        assert_eq!(unsafe { instructions::vmptrst() }, u64::MAX);

        drop(guard1);
        // SAFETY: `guard2` still keeps every CPU in VMX root operation, and no
        // VMCS has been made current.
        assert_eq!(unsafe { instructions::vmptrst() }, u64::MAX);

        drop(guard2);
        let irq_guard = crate::irq::disable_local();
        let state = VMX_CPU_STATE.get_with(&irq_guard);
        assert!(!state.lock().enabled());
        let cr4 = Cr4::read_raw();
        assert_eq!(cr4 & Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits(), 0);
    }
}
