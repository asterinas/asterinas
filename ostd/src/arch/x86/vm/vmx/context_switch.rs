// SPDX-License-Identifier: MPL-2.0

use super::super::types::VcpuRegs;
use crate::irq::DisabledLocalIrqGuard;

/// Enters or resumes the guest and returns after a VM exit.
///
/// # Safety
///
/// 1. VMX must be enabled with an exclusively owned current VMCS whose launch
///    state matches `is_launched`.
/// 2. The VMCS must isolate guest memory and privileged operations, and restore
///    the current host's state, including RIP at `vm_exit_handler_virtaddr()`.
///    All referenced resources must remain alive through the VM exit.
/// 3. The caller must save and restore state not managed by VMX before allowing
///    interrupts or scheduling on this CPU.
pub(super) unsafe fn vcpu_run(
    regs: &mut VcpuRegs,
    is_launched: bool,
    _irq_guard: &DisabledLocalIrqGuard,
) -> u64 {
    // SAFETY: The caller ensures the VMX execution contract. The exclusive
    // reference also provides valid register storage for the assembly routine.
    unsafe { __rkvm_vcpu_run(regs, u64::from(is_launched)) }
}

pub(in crate::arch::vm) fn vm_exit_handler_virtaddr() -> usize {
    __rkvm_vm_exit_handler as *const () as usize
}

unsafe extern "C" {
    fn __rkvm_vcpu_run(regs: *mut VcpuRegs, launched: u64) -> u64;
    fn __rkvm_vm_exit_handler();
}

core::arch::global_asm!(
    include_str!("context_switch.S"),
    host_rsp = const x86::vmx::vmcs::host::RSP,
    rax = const core::mem::offset_of!(VcpuRegs, rax),
    rbx = const core::mem::offset_of!(VcpuRegs, rbx),
    rcx = const core::mem::offset_of!(VcpuRegs, rcx),
    rdx = const core::mem::offset_of!(VcpuRegs, rdx),
    rsi = const core::mem::offset_of!(VcpuRegs, rsi),
    rdi = const core::mem::offset_of!(VcpuRegs, rdi),
    rbp = const core::mem::offset_of!(VcpuRegs, rbp),
    r8 = const core::mem::offset_of!(VcpuRegs, r8),
    r9 = const core::mem::offset_of!(VcpuRegs, r9),
    r10 = const core::mem::offset_of!(VcpuRegs, r10),
    r11 = const core::mem::offset_of!(VcpuRegs, r11),
    r12 = const core::mem::offset_of!(VcpuRegs, r12),
    r13 = const core::mem::offset_of!(VcpuRegs, r13),
    r14 = const core::mem::offset_of!(VcpuRegs, r14),
    r15 = const core::mem::offset_of!(VcpuRegs, r15),
);
