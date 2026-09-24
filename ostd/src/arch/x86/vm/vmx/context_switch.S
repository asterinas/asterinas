/* SPDX-License-Identifier: MPL-2.0 */

.text
.code64

.global __rkvm_vcpu_run
.type __rkvm_vcpu_run, @function

# Arguments: rdi = regs_ptr, rsi = launched (bool/u64).
__rkvm_vcpu_run:
    # Save host callee-saved registers according to the System V AMD64 ABI.
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15

    # Save the guest register pointer.
    push rdi

    # Save host rsp to the VMCS
    # so that we can restore host registers after a VM exit.
    mov rdx, {host_rsp}
    vmwrite rdx, rsp

    # Save the launched flag to the stack.
    push rsi

    # Restore guest registers from VcpuRegs.
    mov rax, [rdi + {rax}]
    mov rbx, [rdi + {rbx}]
    mov rcx, [rdi + {rcx}]
    mov rdx, [rdi + {rdx}]
    mov rsi, [rdi + {rsi}]
    # Skip rdi.
    mov rbp, [rdi + {rbp}]
    # Skip rsp.
    mov r8,  [rdi + {r8}]
    mov r9,  [rdi + {r9}]
    mov r10, [rdi + {r10}]
    mov r11, [rdi + {r11}]
    mov r12, [rdi + {r12}]
    mov r13, [rdi + {r13}]
    mov r14, [rdi + {r14}]
    mov r15, [rdi + {r15}]
    mov rdi, [rdi + {rdi}]  # Restore rdi last.

    # Check whether to use VMLAUNCH or VMRESUME.
    cmp qword ptr [rsp], 0
    jne rkvm_doresume

rkvm_dolaunch:
    vmlaunch
    jmp rkvm_launchfail

rkvm_doresume:
    vmresume
    jmp rkvm_launchfail

# This is where host rip should point.
.global __rkvm_vm_exit_handler
__rkvm_vm_exit_handler:
    # Restore the guest register pointer.
    # After XCHG, [rsp] holds guest rdi, and rdi holds the guest register pointer.
    xchg rdi, [rsp]

    # Save guest registers to VcpuRegs.
    mov [rdi + {rax}], rax
    mov [rdi + {rbx}], rbx
    mov [rdi + {rcx}], rcx
    mov [rdi + {rdx}], rdx
    mov [rdi + {rsi}], rsi
    # Skip rdi.
    mov [rdi + {rbp}], rbp
    # Skip rsp.
    mov [rdi + {r8}], r8
    mov [rdi + {r9}], r9
    mov [rdi + {r10}], r10
    mov [rdi + {r11}], r11
    mov [rdi + {r12}], r12
    mov [rdi + {r13}], r13
    mov [rdi + {r14}], r14
    mov [rdi + {r15}], r15

    pop rax  # Get guest rdi.
    mov [rdi + {rdi}], rax  # Save guest rdi.

    # Restore host registers.
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp

    # Return 0 after a successful VM exit.
    xor rax, rax
    ret

rkvm_launchfail:
    # Failure path.
    pop rax  # Discard the launched flag.
    pop rax  # Discard the guest register pointer.
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp

    # Return 1 as the error code for simplicity.
    mov rax, 1
    ret

.size __rkvm_vcpu_run, .-__rkvm_vcpu_run
