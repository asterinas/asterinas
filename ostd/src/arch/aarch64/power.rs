// SPDX-License-Identifier: MPL-2.0

//! Power management via PSCI (Power State Coordination Interface).
//!
//! PSCI is the ARM standard interface for power management operations
//! such as CPU power state control and system power off/reset.
//!
//! The conduit method (HVC or SMC) is determined from the Device Tree's
//! `/psci` node's `method` property, matching Linux's behavior.
//!
//! Reference: ARM DEN 0022 (PSCI specification)

use spin::Once;

use crate::{
    arch::boot::DEVICE_TREE,
    power::{ExitCode, inject_poweroff_handler, inject_restart_handler},
};

/// PSCI function IDs (v0.2)
/// Reference: Linux include/uapi/linux/psci.h
const PSCI_0_2_FN_SYSTEM_OFF: u64 = 0x84000008;
const PSCI_0_2_FN_SYSTEM_RESET: u64 = 0x84000009;

/// PSCI conduit method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PsciConduit {
    /// Use HVC (HyperVisor Call) - for QEMU virt without EL2/EL3
    Hvc,
    /// Use SMC (Secure Monitor Call) - for systems with EL3/EL2
    Smc,
}

/// Detected PSCI conduit method from device tree
static PSCI_CONDUIT: Once<PsciConduit> = Once::new();

/// Invoke PSCI function using the detected conduit method.
///
/// # Safety
///
/// This function performs an HVC or SMC call which traps to hypervisor/emulator.
/// The caller must ensure the function ID is valid.
#[inline]
unsafe fn psci_call(fn_id: u64, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let mut ret: u64;

    match PSCI_CONDUIT.get().unwrap_or(&PsciConduit::Hvc) {
        PsciConduit::Hvc => {
            // SAFETY: HVC traps to hypervisor/emulator, matching detected conduit.
            unsafe {
                core::arch::asm!(
                    "hvc #0",
                    inout("x0") fn_id => ret,
                    in("x1") arg0,
                    in("x2") arg1,
                    in("x3") arg2,
                    // HVC may clobber x4-x17
                    out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                    out("x8") _, out("x9") _, out("x10") _, out("x11") _,
                    out("x12") _, out("x13") _, out("x14") _, out("x15") _,
                    out("x16") _, out("x17") _,
                );
            }
        }
        PsciConduit::Smc => {
            // SAFETY: SMC traps to secure monitor, matching detected conduit.
            unsafe {
                core::arch::asm!(
                    "smc #0",
                    inout("x0") fn_id => ret,
                    in("x1") arg0,
                    in("x2") arg1,
                    in("x3") arg2,
                    // SMC may clobber x4-x17
                    out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                    out("x8") _, out("x9") _, out("x10") _, out("x11") _,
                    out("x12") _, out("x13") _, out("x14") _, out("x15") _,
                    out("x16") _, out("x17") _,
                );
            }
        }
    }

    ret
}

fn try_poweroff(code: ExitCode) {
    // PSCI SYSTEM_OFF does not take any arguments
    // The exit code is passed to the handler but PSCI doesn't use it
    let _ = code;

    // SAFETY: PSCI SYSTEM_OFF is a standard power management call
    unsafe {
        psci_call(PSCI_0_2_FN_SYSTEM_OFF, 0, 0, 0);
    }
}

fn try_restart(code: ExitCode) {
    // PSCI SYSTEM_RESET does not take any arguments
    // The exit code is passed to the handler but PSCI doesn't use it
    let _ = code;

    // SAFETY: PSCI SYSTEM_RESET is a standard power management call
    unsafe {
        psci_call(PSCI_0_2_FN_SYSTEM_RESET, 0, 0, 0);
    }
}

/// Detect PSCI conduit method from device tree.
///
/// Reads `/psci` node's `method` property:
/// - "hvc" → use HVC (QEMU virt without EL2/EL3)
/// - "smc" → use SMC (systems with EL3/EL2)
///
/// Falls back to HVC if method not specified (QEMU virt default).
fn detect_psci_conduit() -> PsciConduit {
    let fdt = DEVICE_TREE.get();

    if let Some(fdt) = fdt
        && let Some(psci_node) = fdt.find_node("/psci")
        && let Some(method_prop) = psci_node.property("method")
        && let Some(method) = method_prop.as_str()
    {
        crate::early_println!("PSCI conduit method from FDT: {}", method);
        match method {
            "hvc" => return PsciConduit::Hvc,
            "smc" => return PsciConduit::Smc,
            _ => {
                crate::early_println!("WARNING: Unknown PSCI method '{}', using HVC", method);
            }
        }
    }

    // Default to HVC (QEMU virt default without virtualization)
    crate::early_println!("PSCI conduit method: HVC (default)");
    PsciConduit::Hvc
}

pub(super) fn init() {
    // Detect conduit method from device tree
    let conduit = detect_psci_conduit();
    PSCI_CONDUIT.call_once(|| conduit);

    inject_poweroff_handler(try_poweroff);
    inject_restart_handler(try_restart);
}
