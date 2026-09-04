// SPDX-License-Identifier: MPL-2.0

//! Power management.
//!
//! This implements ARM Power State Coordination Interface (PSCI).
//!
//! Reference: <https://developer.arm.com/documentation/den0022/fb/>

use core::arch::asm;

use fdt::node::FdtNode;
use spin::Once;

use crate::{
    arch::boot::DEVICE_TREE,
    power::{ExitCode, inject_poweroff_handler, inject_restart_handler},
};

#[derive(Debug)]
enum PsciMethod {
    Smc,
    Hvc,
}

impl PsciMethod {
    fn parse(node: &FdtNode) -> Option<Self> {
        let method = node.property("method")?.as_str()?;
        match method {
            "smc" => Some(Self::Smc),
            "hvc" => Some(Self::Hvc),
            _ => None,
        }
    }
}

static PSCI_METHOD: Once<PsciMethod> = Once::new();

#[repr(u32)]
enum PsciFunc {
    SystemOff = 0x8400_0008,
    SystemReset = 0x8400_0009,
}

fn try_poweroff(_code: ExitCode) {
    psci_call(PsciFunc::SystemOff);
}

fn try_restart(_code: ExitCode) {
    psci_call(PsciFunc::SystemReset);
}

fn psci_call(func_id: PsciFunc) {
    // If possible, keep this method panic-free because it may be called by the panic handler.
    let Some(psci_method) = PSCI_METHOD.get() else {
        return;
    };

    // For the SMC Calling Convention (SMCCC) version 1.1 and higher, we only need to clobber
    // registers `x0`-`x3`. Since we don't check the SMCC version, we clobber registers `x0`-`x17`,
    // which are required by SMCC v1.0.
    macro_rules! asm_smccc {
        ($insn:literal, $func_id:expr) => {
            asm!(
                $insn,
                in("w0") $func_id,
                lateout("x0") _, out("x1") _, out("x2") _, out("x3") _, out("x4") _, out("x5") _,
                out("x6") _, out("x7") _, out("x8") _, out("x9") _, out("x10") _, out("x11") _,
                out("x12") _, out("x13") _, out("x14") _, out("x15") _, out("x16") _, out("x17") _,
            )
        }
    }

    // SAFETY: We've checked that PSCI exists vis the device tree. Then it is safe to invoke a PCSI
    // function using the correct calling convention.
    unsafe {
        match psci_method {
            PsciMethod::Smc => asm_smccc!("smc #0", func_id as u32),
            PsciMethod::Hvc => asm_smccc!("hvc #0", func_id as u32),
        }
    }
}

pub(super) fn init() {
    // Reference: <https://www.kernel.org/doc/Documentation/devicetree/bindings/arm/psci.txt>
    const FDT_COMPATIBLE: &[&str] = &["arm,psci-0.2", "arm,psci-1.0"];

    let device_tree = DEVICE_TREE.get().unwrap();

    let Some(psci_node) = device_tree.find_compatible(FDT_COMPATIBLE) else {
        return;
    };
    let Some(psci_method) = PsciMethod::parse(&psci_node) else {
        crate::warn!("PSCI node has an invalid method");
        return;
    };

    crate::info!("PSCI detected: {:?}", psci_method);

    PSCI_METHOD.call_once(|| psci_method);
    inject_poweroff_handler(try_poweroff);
    inject_restart_handler(try_restart);
}
