// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use x86_64::registers::control::Cr4;

use super::vmx::{
    self,
    test_support::{self, FailurePoint},
};
use crate::{
    cpu::{PinCurrentCpu, num_cpus},
    prelude::*,
    task::{Task, TaskOptions, disable_preempt},
};

const CR4_VMXE: u64 = 1 << 13;

struct TestCleanup;

impl TestCleanup {
    fn new() -> Self {
        test_support::clear_failures();
        test_support::recover().unwrap();
        Self
    }
}

impl Drop for TestCleanup {
    fn drop(&mut self) {
        test_support::clear_failures();
        let _ = test_support::recover();
    }
}

fn acquire_vmx_for_test() -> vmx::VmxGuard {
    vmx::acquire_vmx().expect("this KTest requires working VMX support")
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let start = crate::arch::read_tsc();
    let timeout = crate::arch::tsc_freq().saturating_mul(5).max(1);

    while !condition() {
        assert!(
            crate::arch::read_tsc().wrapping_sub(start) < timeout,
            "timed out waiting for VMX KTest tasks"
        );
        Task::yield_now();
    }
}

#[ktest]
fn lifecycle_is_shared_and_restores_the_platform() {
    let _cleanup = TestCleanup::new();
    let preempt_guard = disable_preempt();
    let original_cr4 = Cr4::read_raw();

    let first = acquire_vmx_for_test();
    assert_eq!(test_support::active_guard_count(), 1);
    assert_eq!(test_support::enabled_cpu_count(), num_cpus());
    assert_eq!(test_support::allocated_region_count(), num_cpus());
    assert_ne!(Cr4::read_raw() & CR4_VMXE, 0);

    let second = acquire_vmx_for_test();
    assert_eq!(test_support::active_guard_count(), 2);
    drop(first);
    assert_eq!(test_support::active_guard_count(), 1);
    assert_eq!(test_support::enabled_cpu_count(), num_cpus());

    drop(second);
    assert_eq!(test_support::active_guard_count(), 0);
    assert_eq!(test_support::enabled_cpu_count(), 0);
    assert_eq!(test_support::allocated_region_count(), 0);
    assert_eq!(Cr4::read_raw(), original_cr4);

    drop(acquire_vmx_for_test());
    assert_eq!(test_support::enabled_cpu_count(), 0);
    assert_eq!(Cr4::read_raw(), original_cr4);
    drop(preempt_guard);
}

#[ktest]
fn concurrent_guards_do_not_disable_vmx_early() {
    const NUM_WORKERS: usize = 4;

    let _cleanup = TestCleanup::new();
    let anchor = acquire_vmx_for_test();
    let acquired = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));

    for _ in 0..NUM_WORKERS {
        let acquired = acquired.clone();
        let completed = completed.clone();
        let failures = failures.clone();
        let release = release.clone();
        TaskOptions::new(move || {
            let Ok(guard) = vmx::acquire_vmx() else {
                failures.fetch_add(1, Ordering::Release);
                completed.fetch_add(1, Ordering::Release);
                return;
            };
            acquired.fetch_add(1, Ordering::Release);
            while !release.load(Ordering::Acquire) {
                Task::yield_now();
            }
            drop(guard);
            completed.fetch_add(1, Ordering::Release);
        })
        .data(())
        .spawn()
        .unwrap();
    }

    wait_until(|| {
        acquired.load(Ordering::Acquire) + failures.load(Ordering::Acquire) == NUM_WORKERS
    });
    assert_eq!(failures.load(Ordering::Acquire), 0);
    assert_eq!(test_support::active_guard_count(), NUM_WORKERS + 1);

    drop(anchor);
    assert_eq!(test_support::active_guard_count(), NUM_WORKERS);
    assert_eq!(test_support::enabled_cpu_count(), num_cpus());

    release.store(true, Ordering::Release);
    wait_until(|| completed.load(Ordering::Acquire) == NUM_WORKERS);
    assert_eq!(test_support::active_guard_count(), 0);
    assert_eq!(test_support::enabled_cpu_count(), 0);
    assert_eq!(test_support::allocated_region_count(), 0);
}

#[ktest]
fn partial_vmxon_failure_is_rolled_back() {
    let _cleanup = TestCleanup::new();
    assert!(num_cpus() > 1, "this KTest requires an SMP VM");
    let preempt_guard = disable_preempt();
    let original_cr4 = Cr4::read_raw();

    test_support::inject_failure(FailurePoint::Vmxon, preempt_guard.current_cpu());
    assert!(vmx::acquire_vmx().is_err());
    assert_eq!(test_support::active_guard_count(), 0);
    assert_eq!(test_support::enabled_cpu_count(), 0);
    assert_eq!(test_support::allocated_region_count(), 0);
    assert!(!test_support::is_poisoned());
    assert_eq!(Cr4::read_raw(), original_cr4);

    test_support::clear_failures();
    drop(acquire_vmx_for_test());
    assert_eq!(Cr4::read_raw(), original_cr4);
    drop(preempt_guard);
}

#[ktest]
fn vmxoff_failure_poisons_the_lifecycle_and_preserves_resources() {
    let _cleanup = TestCleanup::new();
    assert!(num_cpus() > 1, "this KTest requires an SMP VM");
    let preempt_guard = disable_preempt();
    let original_cr4 = Cr4::read_raw();

    let guard = acquire_vmx_for_test();
    test_support::inject_failure(FailurePoint::Vmxoff, preempt_guard.current_cpu());
    drop(guard);

    assert_eq!(test_support::active_guard_count(), 0);
    assert!(test_support::is_poisoned());
    assert_eq!(test_support::enabled_cpu_count(), 1);
    assert_eq!(test_support::allocated_region_count(), 1);
    assert!(vmx::acquire_vmx().is_err());

    test_support::clear_failures();
    test_support::recover().unwrap();
    assert!(!test_support::is_poisoned());
    assert_eq!(test_support::enabled_cpu_count(), 0);
    assert_eq!(test_support::allocated_region_count(), 0);
    assert_eq!(Cr4::read_raw(), original_cr4);
    drop(preempt_guard);
}
