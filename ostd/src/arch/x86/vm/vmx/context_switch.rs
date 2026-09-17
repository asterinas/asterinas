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
    r#"
    .text
    .code64

    .global __rkvm_vcpu_run
    .type __rkvm_vcpu_run, @function

    # args: rdi = regs_ptr, rsi = launched (bool/u64)
    __rkvm_vcpu_run:
        # Save Callee-Saved Host Registers (according to System V AMD64 ABI)
        push rbp
        push rbx
        push r12
        push r13
        push r14
        push r15

        # save guest regs pointer
        push rdi

        # Save Host RSP to VMCS
        # so that we can restore host regs after VM Exit
        mov rdx, {host_rsp}
        vmwrite rdx, rsp

        # save launched flag to stack
        push rsi

        # restore guest registers from VcpuRegs struct
        mov rax, [rdi + {rax}]
        mov rbx, [rdi + {rbx}]
        mov rcx, [rdi + {rcx}]
        mov rdx, [rdi + {rdx}]
        mov rsi, [rdi + {rsi}]
        ## skip rdi
        mov rbp, [rdi + {rbp}]
        ## skip rsp
        mov r8,  [rdi + {r8}]
        mov r9,  [rdi + {r9}]
        mov r10, [rdi + {r10}]
        mov r11, [rdi + {r11}]
        mov r12, [rdi + {r12}]
        mov r13, [rdi + {r13}]
        mov r14, [rdi + {r14}]
        mov r15, [rdi + {r15}]
        mov rdi, [rdi + {rdi}]  # restore rdi last

        # Check if we should VMLAUNCH or VMRESUME
        cmp qword ptr [rsp], 0
        jne .Lrkvm_doresume

    .Lrkvm_dolaunch:
        vmlaunch
        jmp .Lrkvm_launchfail

    .Lrkvm_doresume:
        vmresume
        jmp .Lrkvm_launchfail

    # This is where HOST_RIP should point
    .global __rkvm_vm_exit_handler
    __rkvm_vm_exit_handler:
        # restore guest regs struct pointer
        # after xchg [rsp] = guest rdi value; rdi = host rdi val = guest regs ptr
        xchg rdi, [rsp]

        # Save guest registers to VcpuRegs struct
        mov [rdi + {rax}], rax
        mov [rdi + {rbx}], rbx
        mov [rdi + {rcx}], rcx
        mov [rdi + {rdx}], rdx
        mov [rdi + {rsi}], rsi
        ## skip rdi
        mov [rdi + {rbp}], rbp
        ## skip rsp
        mov [rdi + {r8}], r8
        mov [rdi + {r9}], r9
        mov [rdi + {r10}], r10
        mov [rdi + {r11}], r11
        mov [rdi + {r12}], r12
        mov [rdi + {r13}], r13
        mov [rdi + {r14}], r14
        mov [rdi + {r15}], r15

        pop rax  # get guest rdi value
        mov [rdi + {rdi}], rax  # save guest rdi value

        # Restore Host Registers
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbx
        pop rbp

        # Return 0 (Success/Exit occurred)
        xor rax, rax
        ret

    .Lrkvm_launchfail:
        # Failure path
        pop rax  # discard launched flag
        pop rax  # discard guest regs pointer
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbx
        pop rbp

        # Return error code (just 1 for simplicity)
        mov rax, 1
        ret

    .size __rkvm_vcpu_run, .-__rkvm_vcpu_run
    "#,
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
