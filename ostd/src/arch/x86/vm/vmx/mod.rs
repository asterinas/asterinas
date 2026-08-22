// SPDX-License-Identifier: MPL-2.0

//! Intel VMX platform lifecycle management.

mod instructions;

#[cfg(all(ktest, feature = "vmx_ktest"))]
use core::sync::atomic::AtomicU32;
use core::{
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};

use x86::msr::rdmsr;
use x86_64::registers::control::{Cr0, Cr4};

use crate::{
    Error,
    cpu::{CpuId, CpuSet, all_cpus},
    cpu_local, error,
    irq::InterruptLevel,
    mm::{Frame, FrameAllocOptions, PAGE_SIZE, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, Mutex, SpinLock},
};

const IA32_FEATURE_CONTROL: u32 = 0x3a;
const IA32_VMX_BASIC: u32 = 0x480;
const IA32_VMX_CR0_FIXED0: u32 = 0x486;
const IA32_VMX_CR0_FIXED1: u32 = 0x487;
const IA32_VMX_CR4_FIXED0: u32 = 0x488;
const IA32_VMX_CR4_FIXED1: u32 = 0x489;
const CR4_VMXE: u64 = 1 << 13;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

const REQUEST_IDLE: u8 = 0;
const ENABLE_PENDING: u8 = 1;
const ENABLE_RUNNING: u8 = 2;
const ENABLE_SUCCEEDED: u8 = 3;
const ENABLE_FAILED: u8 = 4;
const DISABLE_PENDING: u8 = 5;
const DISABLE_RUNNING: u8 = 6;
const DISABLE_SUCCEEDED: u8 = 7;
const DISABLE_FAILED: u8 = 8;
const REQUEST_CANCELLED: u8 = 9;

static VMX_GUARD_STATE: Mutex<VmxGuardState> = Mutex::new(VmxGuardState::new());

#[cfg(all(ktest, feature = "vmx_ktest"))]
const NO_FAILURE_CPU: u32 = u32::MAX;
#[cfg(all(ktest, feature = "vmx_ktest"))]
static FAIL_VMXON_CPU: AtomicU32 = AtomicU32::new(NO_FAILURE_CPU);
#[cfg(all(ktest, feature = "vmx_ktest"))]
static FAIL_VMXOFF_CPU: AtomicU32 = AtomicU32::new(NO_FAILURE_CPU);

cpu_local! {
    static VMX_CPU_STATE: SpinLock<VmxCpuState, LocalIrqDisabled> =
        SpinLock::new(VmxCpuState::new());
    static VMX_REQUEST: AtomicU8 = AtomicU8::new(REQUEST_IDLE);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VmxCpuPhase {
    Disabled,
    Prepared,
    Enabled,
}

struct VmxCpuState {
    phase: VmxCpuPhase,
    region: Option<Frame<()>>,
    original_cr4: u64,
    last_error: Option<Error>,
}

impl VmxCpuState {
    const fn new() -> Self {
        Self {
            phase: VmxCpuPhase::Disabled,
            region: None,
            original_cr4: 0,
            last_error: None,
        }
    }
}

struct EnableError {
    error: Error,
    cleanup_complete: bool,
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

/// Keeps VMX operation enabled while the guard exists.
#[cfg_attr(
    not(all(ktest, feature = "vmx_ktest")),
    expect(
        dead_code,
        reason = "Guest execution will acquire this guard in a follow-up PR"
    )
)]
#[must_use]
pub(crate) struct VmxGuard {
    _private: (),
}

/// Acquires a lease on the VMX platform lifecycle.
#[cfg_attr(
    not(all(ktest, feature = "vmx_ktest")),
    expect(
        dead_code,
        reason = "Guest execution will acquire this guard in a follow-up PR"
    )
)]
pub(crate) fn acquire_vmx() -> Result<VmxGuard> {
    if !InterruptLevel::current().is_task_context()
        || !crate::arch::irq::is_local_enabled()
        || crate::smp::IPI_SENDER.get().is_none()
    {
        return Err(Error::InvalidArgs);
    }

    let mut state = VMX_GUARD_STATE.lock();
    if state.poisoned {
        return Err(Error::InvalidArgs);
    }
    if state.active_guards == usize::MAX {
        return Err(Error::NotEnoughResources);
    }

    if !state.enabled {
        match enable_vmx_on_all_cpus() {
            Ok(()) => state.enabled = true,
            Err(enable_error) => {
                state.enabled = !enable_error.cleanup_complete;
                state.poisoned = !enable_error.cleanup_complete;
                return Err(enable_error.error);
            }
        }
    }
    state.active_guards += 1;

    Ok(VmxGuard { _private: () })
}

impl Drop for VmxGuard {
    fn drop(&mut self) {
        debug_assert!(InterruptLevel::current().is_task_context());
        debug_assert!(crate::arch::irq::is_local_enabled());

        let mut state = VMX_GUARD_STATE.lock();
        if state.active_guards == 0 {
            error!("VMX guard state underflow");
            return;
        }
        state.active_guards -= 1;

        if state.active_guards != 0 {
            return;
        }

        match disable_vmx_on_all_cpus() {
            Ok(()) => state.enabled = false,
            Err(err) => {
                error!("failed to disable VMX on all CPUs: {:?}", err);
                state.poisoned = true;
            }
        }
    }
}

fn enable_vmx_on_all_cpus() -> core::result::Result<(), EnableError> {
    prepare_vmxon_regions().map_err(|error| EnableError {
        error,
        cleanup_complete: true,
    })?;

    let targets = CpuSet::new_full();
    let enable_completed = dispatch_request(
        &targets,
        ENABLE_PENDING,
        ENABLE_RUNNING,
        ENABLE_SUCCEEDED,
        enable_vmx_on_current_cpu,
    );
    if enable_completed {
        reset_requests(&targets);
        return Ok(());
    }

    let enable_error = first_request_error(&targets).unwrap_or(Error::IoError);
    reset_requests(&targets);
    cleanup_prepared_regions();

    let rollback_targets = enabled_cpus();
    if rollback_targets.is_empty() {
        return Err(EnableError {
            error: enable_error,
            cleanup_complete: true,
        });
    }

    let rollback_completed = dispatch_request(
        &rollback_targets,
        DISABLE_PENDING,
        DISABLE_RUNNING,
        DISABLE_SUCCEEDED,
        disable_vmx_on_current_cpu,
    );
    reset_requests(&rollback_targets);
    cleanup_prepared_regions();

    Err(EnableError {
        error: enable_error,
        cleanup_complete: rollback_completed && enabled_cpus().is_empty(),
    })
}

#[cfg_attr(
    not(all(ktest, feature = "vmx_ktest")),
    expect(
        dead_code,
        reason = "Only the follow-up user of VmxGuard will make its Drop path live"
    )
)]
fn disable_vmx_on_all_cpus() -> Result<()> {
    let targets = enabled_cpus();
    if targets.is_empty() {
        return Err(Error::InvalidArgs);
    }

    let completed = dispatch_request(
        &targets,
        DISABLE_PENDING,
        DISABLE_RUNNING,
        DISABLE_SUCCEEDED,
        disable_vmx_on_current_cpu,
    );
    let error = (!completed).then(|| first_request_error(&targets).unwrap_or(Error::IoError));
    reset_requests(&targets);
    cleanup_prepared_regions();

    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn prepare_vmxon_regions() -> Result<()> {
    for cpu in all_cpus() {
        let region = match FrameAllocOptions::new().alloc_frame() {
            Ok(region) => region,
            Err(error) => {
                cleanup_prepared_regions();
                return Err(error);
            }
        };

        let mut state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
        if state.phase != VmxCpuPhase::Disabled || state.region.is_some() {
            drop(state);
            cleanup_prepared_regions();
            return Err(Error::InvalidArgs);
        }
        state.phase = VmxCpuPhase::Prepared;
        state.region = Some(region);
        state.last_error = None;
    }
    Ok(())
}

fn cleanup_prepared_regions() {
    for cpu in all_cpus() {
        let region = {
            let mut state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
            if state.phase != VmxCpuPhase::Prepared {
                continue;
            }
            state.phase = VmxCpuPhase::Disabled;
            state.original_cr4 = 0;
            state.last_error = None;
            state.region.take()
        };
        drop(region);
    }
}

fn enabled_cpus() -> CpuSet {
    let mut enabled = CpuSet::new_empty();
    for cpu in all_cpus() {
        if VMX_CPU_STATE.get_on_cpu(cpu).lock().phase == VmxCpuPhase::Enabled {
            enabled.add(cpu);
        }
    }
    enabled
}

fn dispatch_request(
    targets: &CpuSet,
    pending: u8,
    running: u8,
    succeeded: u8,
    handler: fn(),
) -> bool {
    for cpu in targets.iter() {
        let request = VMX_REQUEST.get_on_cpu(cpu);
        debug_assert_eq!(request.load(Ordering::Relaxed), REQUEST_IDLE);
        request.store(pending, Ordering::Release);
    }

    crate::smp::inter_processor_call(targets, handler);
    wait_for_requests(targets, pending, running, succeeded)
}

fn wait_for_requests(targets: &CpuSet, pending: u8, running: u8, succeeded: u8) -> bool {
    let start = crate::arch::read_tsc();
    let timeout_cycles = duration_to_tsc_cycles(REQUEST_TIMEOUT);

    loop {
        if targets.iter().all(|cpu| {
            let status = VMX_REQUEST.get_on_cpu(cpu).load(Ordering::Acquire);
            status != pending && status != running
        }) {
            break;
        }

        if crate::arch::read_tsc().wrapping_sub(start) >= timeout_cycles {
            for cpu in targets.iter() {
                let _ = VMX_REQUEST.get_on_cpu(cpu).compare_exchange(
                    pending,
                    REQUEST_CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            while targets
                .iter()
                .any(|cpu| VMX_REQUEST.get_on_cpu(cpu).load(Ordering::Acquire) == running)
            {
                core::hint::spin_loop();
            }
            break;
        }

        core::hint::spin_loop();
    }

    targets
        .iter()
        .all(|cpu| VMX_REQUEST.get_on_cpu(cpu).load(Ordering::Acquire) == succeeded)
}

fn duration_to_tsc_cycles(duration: Duration) -> u64 {
    const NANOS_PER_SECOND: u64 = 1_000_000_000;

    let cycles_per_second = crate::arch::tsc_freq();
    let seconds = duration.as_secs().saturating_mul(cycles_per_second);
    let subsecond =
        u64::from(duration.subsec_nanos()).saturating_mul(cycles_per_second) / NANOS_PER_SECOND;
    seconds.saturating_add(subsecond).max(1)
}

fn first_request_error(targets: &CpuSet) -> Option<Error> {
    for cpu in targets.iter() {
        let state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
        if let Some(error) = state.last_error {
            return Some(error);
        }
    }
    None
}

fn reset_requests(targets: &CpuSet) {
    for cpu in targets.iter() {
        let request = VMX_REQUEST.get_on_cpu(cpu);
        debug_assert!(!matches!(
            request.load(Ordering::Acquire),
            ENABLE_PENDING | ENABLE_RUNNING | DISABLE_PENDING | DISABLE_RUNNING
        ));
        request.store(REQUEST_IDLE, Ordering::Release);
    }
}

fn enable_vmx_on_current_cpu() {
    let request = VMX_REQUEST.get_on_cpu(CpuId::current_racy());
    if request
        .compare_exchange(
            ENABLE_PENDING,
            ENABLE_RUNNING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }

    let result = try_enable_vmx_on_current_cpu();
    let status = if result.is_ok() {
        ENABLE_SUCCEEDED
    } else {
        ENABLE_FAILED
    };
    if let Err(error) = result {
        let irq_guard = crate::irq::disable_local();
        VMX_CPU_STATE.get_with(&irq_guard).lock().last_error = Some(error);
    }
    request.store(status, Ordering::Release);
}

fn try_enable_vmx_on_current_cpu() -> Result<()> {
    let irq_guard = crate::irq::disable_local();
    let state = VMX_CPU_STATE.get_with(&irq_guard);
    let (region_paddr, original_cr4) = {
        let state = state.lock();
        if state.phase != VmxCpuPhase::Prepared {
            return Err(Error::InvalidArgs);
        }
        let region_paddr = state.region.as_ref().ok_or(Error::InvalidArgs)?.paddr();
        (region_paddr, Cr4::read_raw())
    };

    let vmx_cr4 = original_cr4 | CR4_VMXE;
    let revision_id = read_and_validate_capability(region_paddr, vmx_cr4)?;
    initialize_vmxon_region(region_paddr, revision_id);

    // SAFETY: `vmx_cr4` preserves the current `CR4` value, adds only
    // `CR4.VMXE`, and has been checked against the VMX fixed-bit MSRs.
    unsafe { Cr4::write_raw(vmx_cr4) };

    #[cfg(all(ktest, feature = "vmx_ktest"))]
    if failure_injected(&FAIL_VMXON_CPU) {
        // SAFETY: Restores the value saved immediately before setting
        // `CR4.VMXE`; this is the same cleanup used for a real VMXON failure.
        unsafe { Cr4::write_raw(original_cr4) };
        return Err(Error::IoError);
    }

    // SAFETY: The capability checks, control-register update, and initialized
    // region above establish the architectural prerequisites for `VMXON`.
    if let Err(error) = unsafe { instructions::vmxon(region_paddr) } {
        // SAFETY: `original_cr4` was read from this CPU immediately before
        // setting `CR4.VMXE`, and no other `CR4` bits have been changed.
        unsafe { Cr4::write_raw(original_cr4) };
        return Err(error);
    }

    let mut state = state.lock();
    state.phase = VmxCpuPhase::Enabled;
    state.original_cr4 = original_cr4;
    state.last_error = None;
    Ok(())
}

fn disable_vmx_on_current_cpu() {
    let request = VMX_REQUEST.get_on_cpu(CpuId::current_racy());
    if request
        .compare_exchange(
            DISABLE_PENDING,
            DISABLE_RUNNING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }

    let result = try_disable_vmx_on_current_cpu();
    let status = if result.is_ok() {
        DISABLE_SUCCEEDED
    } else {
        DISABLE_FAILED
    };
    if let Err(error) = result {
        let irq_guard = crate::irq::disable_local();
        VMX_CPU_STATE.get_with(&irq_guard).lock().last_error = Some(error);
    }
    request.store(status, Ordering::Release);
}

fn try_disable_vmx_on_current_cpu() -> Result<()> {
    let irq_guard = crate::irq::disable_local();
    let state = VMX_CPU_STATE.get_with(&irq_guard);
    let original_cr4 = {
        let state = state.lock();
        if state.phase != VmxCpuPhase::Enabled || state.region.is_none() {
            return Err(Error::InvalidArgs);
        }
        state.original_cr4
    };

    #[cfg(all(ktest, feature = "vmx_ktest"))]
    if failure_injected(&FAIL_VMXOFF_CPU) {
        return Err(Error::IoError);
    }

    // SAFETY: The CPU state is changed to `Enabled` only after a successful
    // `VMXON`, and PR1 does not create or activate any VMCS.
    unsafe { instructions::vmxoff()? };

    // SAFETY: `original_cr4` is the value saved on this CPU immediately
    // before entering VMX operation.
    unsafe { Cr4::write_raw(original_cr4) };

    let mut state = state.lock();
    state.phase = VmxCpuPhase::Prepared;
    state.original_cr4 = 0;
    state.last_error = None;
    Ok(())
}

fn read_and_validate_capability(region_paddr: usize, vmx_cr4: u64) -> Result<u32> {
    const FEATURE_CONTROL_LOCKED: u64 = 1;
    const FEATURE_CONTROL_VMX_OUTSIDE_SMX: u64 = 1 << 2;
    const VMX_BASIC_REGION_SIZE_MASK: u64 = 0x1fff;
    const VMX_BASIC_32BIT_PHYS_ADDR: u64 = 1 << 48;
    const VMX_BASIC_MEMORY_TYPE_MASK: u64 = 0xf;
    const WRITE_BACK_MEMORY_TYPE: u64 = 6;

    let has_vmx =
        crate::arch::cpu::cpuid::cpuid(1, 0).is_some_and(|result| result.ecx & (1 << 5) != 0);
    if !has_vmx {
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

    let required_feature_control = FEATURE_CONTROL_LOCKED | FEATURE_CONTROL_VMX_OUTSIDE_SMX;
    if feature_control & required_feature_control != required_feature_control {
        return Err(Error::AccessDenied);
    }

    let required_size = ((vmx_basic >> 32) & VMX_BASIC_REGION_SIZE_MASK) as usize;
    let memory_type = (vmx_basic >> 50) & VMX_BASIC_MEMORY_TYPE_MASK;
    if required_size == 0
        || required_size > PAGE_SIZE
        || !region_paddr.is_multiple_of(PAGE_SIZE)
        || memory_type != WRITE_BACK_MEMORY_TYPE
    {
        return Err(Error::NotEnoughResources);
    }

    if vmx_basic & VMX_BASIC_32BIT_PHYS_ADDR != 0 {
        let region_end = region_paddr
            .checked_add(PAGE_SIZE - 1)
            .ok_or(Error::NotEnoughResources)?;
        if region_end > u32::MAX as usize {
            return Err(Error::NotEnoughResources);
        }
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

#[cfg(all(ktest, feature = "vmx_ktest"))]
fn failure_injected(failure_cpu: &AtomicU32) -> bool {
    failure_cpu.load(Ordering::Acquire) == u32::from(CpuId::current_racy())
}

fn initialize_vmxon_region(region_paddr: usize, revision_id: u32) {
    let region_ptr = paddr_to_vaddr(region_paddr) as *mut u32;

    // SAFETY: `region_paddr` belongs to the live, exclusively owned frame in
    // the current CPU state. The frame is linearly mapped, page-aligned, and
    // was zero-initialized before this four-byte write.
    unsafe { region_ptr.write(revision_id) };
}

#[cfg(all(ktest, feature = "vmx_ktest"))]
pub(super) mod test_support {
    use super::*;

    #[derive(Clone, Copy)]
    pub(in crate::arch::vm) enum FailurePoint {
        Vmxon,
        Vmxoff,
    }

    pub(in crate::arch::vm) fn inject_failure(point: FailurePoint, cpu: CpuId) {
        let failure_cpu = match point {
            FailurePoint::Vmxon => &FAIL_VMXON_CPU,
            FailurePoint::Vmxoff => &FAIL_VMXOFF_CPU,
        };
        failure_cpu.store(u32::from(cpu), Ordering::Release);
    }

    pub(in crate::arch::vm) fn clear_failures() {
        FAIL_VMXON_CPU.store(NO_FAILURE_CPU, Ordering::Release);
        FAIL_VMXOFF_CPU.store(NO_FAILURE_CPU, Ordering::Release);
    }

    pub(in crate::arch::vm) fn active_guard_count() -> usize {
        VMX_GUARD_STATE.lock().active_guards
    }

    pub(in crate::arch::vm) fn is_poisoned() -> bool {
        VMX_GUARD_STATE.lock().poisoned
    }

    pub(in crate::arch::vm) fn enabled_cpu_count() -> usize {
        all_cpus()
            .filter(|cpu| VMX_CPU_STATE.get_on_cpu(*cpu).lock().phase == VmxCpuPhase::Enabled)
            .count()
    }

    pub(in crate::arch::vm) fn allocated_region_count() -> usize {
        all_cpus()
            .filter(|cpu| VMX_CPU_STATE.get_on_cpu(*cpu).lock().region.is_some())
            .count()
    }

    pub(in crate::arch::vm) fn recover() -> Result<()> {
        clear_failures();

        let mut state = VMX_GUARD_STATE.lock();
        if state.active_guards != 0 {
            return Err(Error::InvalidArgs);
        }
        if state.enabled {
            disable_vmx_on_all_cpus()?;
            state.enabled = false;
        }
        state.poisoned = false;
        Ok(())
    }
}
