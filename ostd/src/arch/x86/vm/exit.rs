// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;
use x86::vmx::vmcs::{guest, ro};

use super::vmx::vmcs::CurrentVmcs;
use crate::{Error, prelude::*, vm::Gpaddr};

/// A VM exit that needs handling by the kernel client.
#[derive(Clone, Copy, Debug)]
pub struct GuestExitInfo {
    /// The basic VMX exit reason.
    pub exit_reason: u32,
    /// The length of the exiting instruction, or zero for non-instruction exits.
    pub instruction_len: u32,
    /// Architecture-specific exit qualification.
    pub exit_qualification: u64,
    /// The guest physical address for an EPT exit, or zero otherwise.
    pub guest_phys_addr: Gpaddr,
    /// The guest instruction pointer at the exit.
    pub guest_rip: usize,
    /// VM-exit interruption information for an exception or NMI, or zero otherwise.
    pub interruption_info: u32,
    /// The exception error code, if indicated by `interruption_info`.
    pub interruption_error_code: u32,
}

/// VMX basic exit reasons.
///
/// See Intel SDM, Vol. 3D, Appendix C, Table C-1
#[expect(non_camel_case_types)]
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VmxExitReason {
    /// An exception selected by the exception bitmap, or an intercepted NMI.
    EXCEPTION_NMI = 0,
    /// An external interrupt with external-interrupt exiting enabled.
    EXTERNAL_INTERRUPT = 1,
    /// An exception during double-fault delivery that was not intercepted.
    TRIPLE_FAULT = 2,
    /// Receipt of an INIT signal.
    INIT = 3,
    /// Receipt of a startup IPI while waiting for SIPI.
    SIPI = 4,
    /// An SMM VM exit caused by an SMI immediately after an I/O instruction retires.
    SMI = 5,
    /// An SMM VM exit caused by an SMI not immediately following I/O instruction retirement.
    OTHER_SMI = 6,
    /// An interrupt window with `RFLAGS.IF` set, no `STI` or `MOV SS` blocking,
    /// and interrupt-window exiting enabled.
    INTERRUPT_WINDOW = 7,
    /// An NMI window with no virtual-NMI or `MOV SS` blocking and NMI-window exiting enabled.
    NMI_WINDOW = 8,
    /// A task switch attempted by the guest.
    TASK_SWITCH = 9,
    /// An attempt to execute `CPUID`.
    CPUID = 10,
    /// An attempt to execute `GETSEC`.
    GETSEC = 11,
    /// An intercepted `HLT` instruction.
    HLT = 12,
    /// An attempt to execute `INVD`.
    INVD = 13,
    /// An intercepted `INVLPG` instruction.
    INVLPG = 14,
    /// An intercepted `RDPMC` instruction.
    RDPMC = 15,
    /// An intercepted `RDTSC` instruction.
    RDTSC = 16,
    /// An attempt to execute `RSM` in SMM.
    RSM = 17,
    /// A `VMCALL` from the guest or the executive monitor.
    VMCALL = 18,
    /// An attempt to execute `VMCLEAR`.
    VMCLEAR = 19,
    /// An attempt to execute `VMLAUNCH`.
    VMLAUNCH = 20,
    /// An attempt to execute `VMPTRLD`.
    VMPTRLD = 21,
    /// An attempt to execute `VMPTRST`.
    VMPTRST = 22,
    /// An attempt to execute `VMREAD`.
    VMREAD = 23,
    /// An attempt to execute `VMRESUME`.
    VMRESUME = 24,
    /// An attempt to execute `VMWRITE`.
    VMWRITE = 25,
    /// An attempt to execute `VMXOFF`.
    VMOFF = 26,
    /// An attempt to execute `VMXON`.
    VMON = 27,
    /// An intercepted access to `CR0`, `CR3`, `CR4`, or `CR8` via `CLTS`, `LMSW`, or `MOV CR`.
    CR_ACCESS = 28,
    /// An intercepted `MOV` to or from a debug register.
    DR_ACCESS = 29,
    /// An I/O instruction intercepted by I/O exiting controls or the I/O bitmaps.
    IO_INSTRUCTION = 30,
    /// An intercepted `RDMSR` instruction.
    MSR_READ = 31,
    /// An intercepted `WRMSR` or `WRMSRNS` instruction.
    MSR_WRITE = 32,
    /// A VM-entry failure caused by invalid guest state.
    INVALID_GUEST_STATE = 33,
    /// A VM-entry failure while loading guest MSRs.
    MSR_LOAD_FAIL = 34,
    /// An intercepted `MWAIT` instruction.
    MWAIT_INSTRUCTION = 36,
    /// A monitor-trap-flag exit enabled by VM controls or injected on VM entry.
    MONITOR_TRAP_FLAG = 37,
    /// An intercepted `MONITOR` instruction.
    MONITOR_INSTRUCTION = 39,
    /// An intercepted `PAUSE` instruction or a pause loop exceeding `PLE_Window`.
    PAUSE_INSTRUCTION = 40,
    /// A machine-check event during VM entry.
    MCE_DURING_VMENTRY = 41,
    /// A virtual TPR priority below the configured threshold while TPR shadowing is enabled.
    TPR_BELOW_THRESHOLD = 43,
    /// An intercepted memory access to the APIC-access page.
    APIC_ACCESS = 44,
    /// A virtual EOI for an interrupt selected by the EOI-exit bitmap.
    VIRTUALIZED_EOI = 45,
    /// An intercepted `LGDT`, `LIDT`, `SGDT`, or `SIDT` instruction.
    GDTR_IDTR = 46,
    /// An intercepted `LLDT`, `LTR`, `SLDT`, or `STR` instruction.
    LDTR_TR = 47,
    /// A guest physical memory access denied by the EPT paging structures.
    EPT_VIOLATION = 48,
    /// A guest physical memory access encountering an invalid EPT entry.
    EPT_MISCONFIG = 49,
    /// An attempt to execute `INVEPT`.
    INVEPT = 50,
    /// An intercepted `RDTSCP` instruction.
    RDTSCP = 51,
    /// Expiration of the VMX preemption timer.
    PREEMPTION_TIMER = 52,
    /// An attempt to execute `INVVPID`.
    INVVPID = 53,
    /// An intercepted `WBINVD` or `WBNOINVD` instruction.
    WBINVD = 54,
    /// An attempt to execute `XSETBV`.
    XSETBV = 55,
    /// A completed write to the virtual-APIC page requiring VMM emulation.
    APIC_WRITE = 56,
    /// An intercepted `RDRAND` instruction.
    RDRAND = 57,
    /// An intercepted `INVPCID` instruction.
    INVPCID = 58,
    /// A `VMFUNC` for a disabled function or a function-specific exit condition.
    VMFUNC = 59,
    /// An `ENCLS` leaf intercepted by the ENCLS-exiting bitmap.
    ENCLS = 60,
    /// An intercepted `RDSEED` instruction.
    RDSEED = 61,
    /// An attempted page-modification log update with the PML index outside `0..=511`.
    PML_FULL = 62,
    /// An `XSAVES` request for state selected by `IA32_XSS` and the XSS-exiting bitmap.
    XSAVES = 63,
    /// An `XRSTORS` request for state selected by `IA32_XSS` and the XSS-exiting bitmap.
    XRSTORS = 64,
    /// A `PCONFIG` leaf intercepted by the PCONFIG-exiting bitmap.
    PCONFIG = 65,
    /// A miss or misconfiguration while looking up sub-page write permissions.
    SPP_EVENT = 66,
    /// An intercepted `UMWAIT` instruction.
    UMWAIT = 67,
    /// An intercepted `TPAUSE` instruction.
    TPAUSE = 68,
    /// An intercepted `LOADIWKEY` instruction.
    LOADIWKEY = 69,
}

/// Reads the current VMCS's exit information.
pub(super) fn exit_info(vmcs: &CurrentVmcs<'_>) -> Result<GuestExitInfo> {
    let raw_reason = vmcs.read(ro::EXIT_REASON)?;
    // A late VM-entry failure reaches the VM-exit handler instead of setting
    // CF/ZF at VMLAUNCH/VMRESUME (Intel SDM, Vol. 3C, Section 26.7).
    if raw_reason & (1 << 31) != 0 {
        return Err(Error::InvalidArgs);
    }
    let reason = (raw_reason & 0xffff) as u32;
    let is_ept = reason == VmxExitReason::EPT_VIOLATION as u32
        || reason == VmxExitReason::EPT_MISCONFIG as u32;
    let interruption_info = if reason == VmxExitReason::EXCEPTION_NMI as u32 {
        vmcs.read(ro::VMEXIT_INTERRUPTION_INFO)? as u32
    } else {
        0
    };
    let interruption_error_code = if interruption_info & (1 << 11) != 0 {
        vmcs.read(ro::VMEXIT_INTERRUPTION_ERR_CODE)? as u32
    } else {
        0
    };
    // The minimal backend exposes these instruction exits. Do not report a
    // stale length for asynchronous events or EPT faults.
    let instruction_len = if matches!(
        VmxExitReason::try_from(reason),
        Ok(VmxExitReason::CPUID
            | VmxExitReason::HLT
            | VmxExitReason::INVD
            | VmxExitReason::INVLPG
            | VmxExitReason::RDPMC
            | VmxExitReason::RDTSC
            | VmxExitReason::VMCALL
            | VmxExitReason::VMCLEAR
            | VmxExitReason::VMLAUNCH
            | VmxExitReason::VMPTRLD
            | VmxExitReason::VMPTRST
            | VmxExitReason::VMREAD
            | VmxExitReason::VMRESUME
            | VmxExitReason::VMWRITE
            | VmxExitReason::VMOFF
            | VmxExitReason::VMON
            | VmxExitReason::CR_ACCESS
            | VmxExitReason::DR_ACCESS
            | VmxExitReason::IO_INSTRUCTION
            | VmxExitReason::MSR_READ
            | VmxExitReason::MSR_WRITE
            | VmxExitReason::MWAIT_INSTRUCTION
            | VmxExitReason::MONITOR_INSTRUCTION
            | VmxExitReason::PAUSE_INSTRUCTION
            | VmxExitReason::WBINVD
            | VmxExitReason::XSETBV)
    ) {
        vmcs.read(ro::VMEXIT_INSTRUCTION_LEN)? as u32
    } else {
        0
    };
    Ok(GuestExitInfo {
        exit_reason: reason,
        instruction_len,
        exit_qualification: vmcs.read(ro::EXIT_QUALIFICATION)? as u64,
        guest_phys_addr: if is_ept {
            vmcs.read(ro::GUEST_PHYSICAL_ADDR_FULL)?
        } else {
            0
        },
        guest_rip: vmcs.read(guest::RIP)?,
        interruption_info,
        interruption_error_code,
    })
}
