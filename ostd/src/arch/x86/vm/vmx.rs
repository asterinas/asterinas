// SPDX-License-Identifier: MPL-2.0

//! VMX (Virtual Machine Extensions) operations for Intel VT-x
//!
//! This module provides wrappers for VMX instructions and VMCS access
#![allow(missing_docs)]
#![expect(
    dead_code,
    reason = "VMX support is being wired in stages, so these wrappers are not all called yet."
)]

use crate::{error::Error, mm::Paddr, prelude::Result};

/*
 * This file contains code derived from the RVM-Tutorial project.
 * Source: https://github.com/equation314/RVM-Tutorial
 */
#[expect(
    non_camel_case_types,
    reason = "VMX names follow Intel SDM terminology, and the catalog includes MSRs reserved for future VMX paths."
)]
#[cfg_attr(
    not(ktest),
    expect(
        dead_code,
        reason = "VMX names follow Intel SDM terminology, and the catalog includes MSRs reserved for future VMX paths."
    )
)]
#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Msr {
    IA32_FEATURE_CONTROL = 0x3a,

    IA32_PAT = 0x277,

    IA32_VMX_BASIC = 0x480,
    IA32_VMX_PINBASED_CTLS = 0x481,
    IA32_VMX_PROCBASED_CTLS = 0x482,
    IA32_VMX_EXIT_CTLS = 0x483,
    IA32_VMX_ENTRY_CTLS = 0x484,
    IA32_VMX_MISC = 0x485,
    IA32_VMX_CR0_FIXED0 = 0x486,
    IA32_VMX_CR0_FIXED1 = 0x487,
    IA32_VMX_CR4_FIXED0 = 0x488,
    IA32_VMX_CR4_FIXED1 = 0x489,
    IA32_VMX_PROCBASED_CTLS2 = 0x48b,
    IA32_VMX_EPT_VPID_CAP = 0x48c,
    IA32_VMX_TRUE_PINBASED_CTLS = 0x48d,
    IA32_VMX_TRUE_PROCBASED_CTLS = 0x48e,
    IA32_VMX_TRUE_EXIT_CTLS = 0x48f,
    IA32_VMX_TRUE_ENTRY_CTLS = 0x490,

    IA32_EFER = 0xc000_0080,
    IA32_STAR = 0xc000_0081,
    IA32_LSTAR = 0xc000_0082,
    IA32_CSTAR = 0xc000_0083,
    IA32_FMASK = 0xc000_0084,

    IA32_FS_BASE = 0xc000_0100,
    IA32_GS_BASE = 0xc000_0101,
    IA32_KERNEL_GSBASE = 0xc000_0102,
}

#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack)
        );
    }
    ((high as u64) << 32) | (low as u64)
}

#[inline]
unsafe fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nostack)
        );
    }
}

impl Msr {
    pub(crate) fn read(&self) -> u64 {
        unsafe { rdmsr(*self as u32) }
    }

    pub(crate) fn write(&self, value: u64) {
        unsafe { wrmsr(*self as u32, value) }
    }
}

// Reads a VMCS field and reports VM-instruction failure through `Result`.
#[inline]
unsafe fn vmread(field: u32) -> Result<u64> {
    let value: u64;
    let mut rflags: u64;
    unsafe {
        core::arch::asm!(
            "vmread {}, {}",
            "pushfq",
            "pop {}",
            out(reg) value,
            in(reg) field as u64,
            out(reg) rflags,
            options(nomem, nostack)
        );
    }

    if (rflags & 1) != 0 {
        return Err(Error::InvalidArgs);
    }
    if (rflags & (1 << 6)) != 0 {
        return Err(Error::InvalidArgs);
    }

    Ok(value)
}

// Writes a VMCS field and reports VM-instruction failure through `Result`.
#[inline]
unsafe fn vmwrite(field: u32, value: u64) -> Result<()> {
    let mut rflags: u64;
    unsafe {
        core::arch::asm!(
            "vmwrite {}, {}",
            "pushfq",
            "pop {}",
            in(reg) field as u64,
            in(reg) value,
            out(reg) rflags,
            options(nostack)
        );
    }

    if (rflags & 1) != 0 {
        return Err(Error::InvalidArgs);
    }
    if (rflags & (1 << 6)) != 0 {
        return Err(Error::InvalidArgs);
    }

    Ok(())
}

macro_rules! vmcs_read {
    ($field_enum: ident, u64) => {
        impl $field_enum {
            pub(crate) fn read(self) -> Result<u64> {
                unsafe { vmread(self as u32) }
            }
        }
    };
    ($field_enum: ident, $ux: ty) => {
        impl $field_enum {
            pub(crate) fn read(self) -> Result<$ux> {
                unsafe { vmread(self as u32).map(|v| v as $ux) }
            }
        }
    };
}

macro_rules! vmcs_write {
    ($field_enum: ident, u64) => {
        impl $field_enum {
            pub(crate) fn write(self, value: u64) -> Result<()> {
                unsafe { vmwrite(self as u32, value) }
            }
        }
    };
    ($field_enum: ident, $ux: ty) => {
        impl $field_enum {
            pub(crate) fn write(self, value: $ux) -> Result<()> {
                unsafe { vmwrite(self as u32, value as u64) }
            }
        }
    };
}

/// 16-Bit Control Fields. (SDM Vol. 3D, Appendix B.1.1)
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::upper_case_acronyms,
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsControl16 {
    /// Virtual-processor identifier (VPID).
    VPID = 0x0,
    /// Posted-interrupt notification vector.
    POSTED_INTERRUPT_NOTIFICATION_VECTOR = 0x2,
    /// EPTP index.
    EPTP_INDEX = 0x4,
}

/// 64-Bit Control Fields. (SDM Vol. 3D, Appendix B.2.1)
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::upper_case_acronyms,
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsControl64 {
    /// Address of I/O bitmap A.
    IO_BITMAP_A_ADDR = 0x2000,
    /// Address of I/O bitmap B.
    IO_BITMAP_B_ADDR = 0x2002,
    /// Address of MSR bitmaps.
    MSR_BITMAPS_ADDR = 0x2004,
    /// VM-exit MSR-store address.
    VMEXIT_MSR_STORE_ADDR = 0x2006,
    /// VM-exit MSR-load address.
    VMEXIT_MSR_LOAD_ADDR = 0x2008,
    /// VM-entry MSR-load address.
    VMENTRY_MSR_LOAD_ADDR = 0x200A,
    /// Executive-VMCS pointer.
    EXECUTIVE_VMCS_PTR = 0x200C,
    /// PML address.
    PML_ADDR = 0x200E,
    /// TSC offset.
    TSC_OFFSET = 0x2010,
    /// Virtual-APIC address.
    VIRT_APIC_ADDR = 0x2012,
    /// APIC-access address.
    APIC_ACCESS_ADDR = 0x2014,
    /// Posted-interrupt descriptor address.
    POSTED_INTERRUPT_DESC_ADDR = 0x2016,
    /// VM-function controls.
    VM_FUNCTION_CONTROLS = 0x2018,
    /// EPT pointer.
    EPTP = 0x201A,
    /// EOI-exit bitmap 0.
    EOI_EXIT0 = 0x201C,
    /// EOI-exit bitmap 1.
    EOI_EXIT1 = 0x201E,
    /// EOI-exit bitmap 2.
    EOI_EXIT2 = 0x2020,
    /// EOI-exit bitmap 3.
    EOI_EXIT3 = 0x2022,
    /// EPTP-list address.
    EPTP_LIST_ADDR = 0x2024,
    /// VMREAD-bitmap address.
    VMREAD_BITMAP_ADDR = 0x2026,
    /// VMWRITE-bitmap address.
    VMWRITE_BITMAP_ADDR = 0x2028,
    /// Virtualization-exception information address.
    VIRT_EXCEPTION_INFO_ADDR = 0x202A,
    /// XSS-exiting bitmap.
    XSS_EXITING_BITMAP = 0x202C,
    /// ENCLS-exiting bitmap.
    ENCLS_EXITING_BITMAP = 0x202E,
    /// Sub-page-permission-table pointer.
    SUBPAGE_PERM_TABLE_PTR = 0x2030,
    /// TSC multiplier.
    TSC_MULTIPLIER = 0x2032,
}
vmcs_read!(VmcsControl64, u64);
vmcs_write!(VmcsControl64, u64);

/// 32-Bit Control Fields. (SDM Vol. 3D, Appendix B.3.1)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsControl32 {
    /// Pin-based VM-execution controls.
    PINBASED_EXEC_CONTROLS = 0x4000,
    /// Primary processor-based VM-execution controls.
    PRIMARY_PROCBASED_EXEC_CONTROLS = 0x4002,
    /// Exception bitmap.
    EXCEPTION_BITMAP = 0x4004,
    /// Page-fault error-code mask.
    PAGE_FAULT_ERR_CODE_MASK = 0x4006,
    /// Page-fault error-code match.
    PAGE_FAULT_ERR_CODE_MATCH = 0x4008,
    /// CR3-target count.
    CR3_TARGET_COUNT = 0x400A,
    /// VM-exit controls.
    VMEXIT_CONTROLS = 0x400C,
    /// VM-exit MSR-store count.
    VMEXIT_MSR_STORE_COUNT = 0x400E,
    /// VM-exit MSR-load count.
    VMEXIT_MSR_LOAD_COUNT = 0x4010,
    /// VM-entry controls.
    VMENTRY_CONTROLS = 0x4012,
    /// VM-entry MSR-load count.
    VMENTRY_MSR_LOAD_COUNT = 0x4014,
    /// VM-entry interruption-information field.
    VMENTRY_INTERRUPTION_INFO_FIELD = 0x4016,
    /// VM-entry exception error code.
    VMENTRY_EXCEPTION_ERR_CODE = 0x4018,
    /// VM-entry instruction length.
    VMENTRY_INSTRUCTION_LEN = 0x401A,
    /// TPR threshold.
    TPR_THRESHOLD = 0x401C,
    /// Secondary processor-based VM-execution controls.
    SECONDARY_PROCBASED_EXEC_CONTROLS = 0x401E,
    /// PLE_Gap.
    PLE_GAP = 0x4020,
    /// PLE_Window.
    PLE_WINDOW = 0x4022,
}
vmcs_read!(VmcsControl32, u32);
vmcs_write!(VmcsControl32, u32);

/// Natural-Width Control Fields. (SDM Vol. 3D, Appendix B.4.1)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsControlNW {
    /// CR0 guest/host mask.
    CR0_GUEST_HOST_MASK = 0x6000,
    /// CR4 guest/host mask.
    CR4_GUEST_HOST_MASK = 0x6002,
    /// CR0 read shadow.
    CR0_READ_SHADOW = 0x6004,
    /// CR4 read shadow.
    CR4_READ_SHADOW = 0x6006,
    /// CR3-target value 0.
    CR3_TARGET_VALUE0 = 0x6008,
    /// CR3-target value 1.
    CR3_TARGET_VALUE1 = 0x600A,
    /// CR3-target value 2.
    CR3_TARGET_VALUE2 = 0x600C,
    /// CR3-target value 3.
    CR3_TARGET_VALUE3 = 0x600E,
}
vmcs_read!(VmcsControlNW, usize);
vmcs_write!(VmcsControlNW, usize);

/// 16-Bit Guest-State Fields. (SDM Vol. 3D, Appendix B.1.2)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsGuest16 {
    /// Guest ES selector.
    ES_SELECTOR = 0x800,
    /// Guest CS selector.
    CS_SELECTOR = 0x802,
    /// Guest SS selector.
    SS_SELECTOR = 0x804,
    /// Guest DS selector.
    DS_SELECTOR = 0x806,
    /// Guest FS selector.
    FS_SELECTOR = 0x808,
    /// Guest GS selector.
    GS_SELECTOR = 0x80a,
    /// Guest LDTR selector.
    LDTR_SELECTOR = 0x80c,
    /// Guest TR selector.
    TR_SELECTOR = 0x80e,
    /// Guest interrupt status.
    INTERRUPT_STATUS = 0x810,
    /// PML index.
    PML_INDEX = 0x812,
}
vmcs_read!(VmcsGuest16, u16);
vmcs_write!(VmcsGuest16, u16);

/// 64-Bit Guest-State Fields. (SDM Vol. 3D, Appendix B.2.3)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsGuest64 {
    /// VMCS link pointer.
    LINK_PTR = 0x2800,
    /// Guest IA32_DEBUGCTL.
    IA32_DEBUGCTL = 0x2802,
    /// Guest IA32_PAT.
    IA32_PAT = 0x2804,
    /// Guest IA32_EFER.
    IA32_EFER = 0x2806,
    /// Guest IA32_PERF_GLOBAL_CTRL.
    IA32_PERF_GLOBAL_CTRL = 0x2808,
    /// Guest PDPTE0.
    PDPTE0 = 0x280A,
    /// Guest PDPTE1.
    PDPTE1 = 0x280C,
    /// Guest PDPTE2.
    PDPTE2 = 0x280E,
    /// Guest PDPTE3.
    PDPTE3 = 0x2810,
    /// Guest IA32_BNDCFGS.
    IA32_BNDCFGS = 0x2812,
    /// Guest IA32_RTIT_CTL.
    IA32_RTIT_CTL = 0x2814,
}
vmcs_read!(VmcsGuest64, u64);
vmcs_write!(VmcsGuest64, u64);

/// 32-Bit Guest-State Fields. (SDM Vol. 3D, Appendix B.3.3)
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::upper_case_acronyms,
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsGuest32 {
    /// Guest ES limit.
    ES_LIMIT = 0x4800,
    /// Guest CS limit.
    CS_LIMIT = 0x4802,
    /// Guest SS limit.
    SS_LIMIT = 0x4804,
    /// Guest DS limit.
    DS_LIMIT = 0x4806,
    /// Guest FS limit.
    FS_LIMIT = 0x4808,
    /// Guest GS limit.
    GS_LIMIT = 0x480A,
    /// Guest LDTR limit.
    LDTR_LIMIT = 0x480C,
    /// Guest TR limit.
    TR_LIMIT = 0x480E,
    /// Guest GDTR limit.
    GDTR_LIMIT = 0x4810,
    /// Guest IDTR limit.
    IDTR_LIMIT = 0x4812,
    /// Guest ES access rights.
    ES_ACCESS_RIGHTS = 0x4814,
    /// Guest CS access rights.
    CS_ACCESS_RIGHTS = 0x4816,
    /// Guest SS access rights.
    SS_ACCESS_RIGHTS = 0x4818,
    /// Guest DS access rights.
    DS_ACCESS_RIGHTS = 0x481A,
    /// Guest FS access rights.
    FS_ACCESS_RIGHTS = 0x481C,
    /// Guest GS access rights.
    GS_ACCESS_RIGHTS = 0x481E,
    /// Guest LDTR access rights.
    LDTR_ACCESS_RIGHTS = 0x4820,
    /// Guest TR access rights.
    TR_ACCESS_RIGHTS = 0x4822,
    /// Guest interruptibility state.
    INTERRUPTIBILITY_STATE = 0x4824,
    /// Guest activity state.
    ACTIVITY_STATE = 0x4826,
    /// Guest SMBASE.
    SMBASE = 0x4828,
    /// Guest IA32_SYSENTER_CS.
    IA32_SYSENTER_CS = 0x482A,
    /// VMX-preemption timer value.
    VMX_PREEMPTION_TIMER_VALUE = 0x482E,
}
vmcs_read!(VmcsGuest32, u32);
vmcs_write!(VmcsGuest32, u32);

/// Natural-Width Guest-State Fields. (SDM Vol. 3D, Appendix B.4.3)
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::upper_case_acronyms,
    non_camel_case_types,
    reason = "VMX names follow Intel SDM terminology."
)]
pub(crate) enum VmcsGuestNW {
    /// Guest CR0.
    CR0 = 0x6800,
    /// Guest CR3.
    CR3 = 0x6802,
    /// Guest CR4.
    CR4 = 0x6804,
    /// Guest ES base.
    ES_BASE = 0x6806,
    /// Guest CS base.
    CS_BASE = 0x6808,
    /// Guest SS base.
    SS_BASE = 0x680A,
    /// Guest DS base.
    DS_BASE = 0x680C,
    /// Guest FS base.
    FS_BASE = 0x680E,
    /// Guest GS base.
    GS_BASE = 0x6810,
    /// Guest LDTR base.
    LDTR_BASE = 0x6812,
    /// Guest TR base.
    TR_BASE = 0x6814,
    /// Guest GDTR base.
    GDTR_BASE = 0x6816,
    /// Guest IDTR base.
    IDTR_BASE = 0x6818,
    /// Guest DR7.
    DR7 = 0x681A,
    /// Guest RSP.
    RSP = 0x681C,
    /// Guest RIP.
    RIP = 0x681E,
    /// Guest RFLAGS.
    RFLAGS = 0x6820,
    /// Guest pending debug exceptions.
    PENDING_DBG_EXCEPTIONS = 0x6822,
    /// Guest IA32_SYSENTER_ESP.
    IA32_SYSENTER_ESP = 0x6824,
    /// Guest IA32_SYSENTER_EIP.
    IA32_SYSENTER_EIP = 0x6826,
}
vmcs_read!(VmcsGuestNW, usize);
vmcs_write!(VmcsGuestNW, usize);

/// 16-Bit Host-State Fields. (SDM Vol. 3D, Appendix B.1.3)
#[derive(Clone, Copy, Debug)]
#[expect(
    non_camel_case_types,
    reason = "VMX names follow Intel SDM terminology."
)]
pub(super) enum VmcsHost16 {
    /// Host ES selector.
    ES_SELECTOR = 0xC00,
    /// Host CS selector.
    CS_SELECTOR = 0xC02,
    /// Host SS selector.
    SS_SELECTOR = 0xC04,
    /// Host DS selector.
    DS_SELECTOR = 0xC06,
    /// Host FS selector.
    FS_SELECTOR = 0xC08,
    /// Host GS selector.
    GS_SELECTOR = 0xC0A,
    /// Host TR selector.
    TR_SELECTOR = 0xC0C,
}
vmcs_write!(VmcsHost16, u16);

/// 64-Bit Host-State Fields. (SDM Vol. 3D, Appendix B.2.4)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(super) enum VmcsHost64 {
    /// Host IA32_PAT.
    IA32_PAT = 0x2C00,
    /// Host IA32_EFER.
    IA32_EFER = 0x2C02,
    /// Host IA32_PERF_GLOBAL_CTRL.
    IA32_PERF_GLOBAL_CTRL = 0x2C04,
}
vmcs_write!(VmcsHost64, u64);

/// 32-Bit Host-State Field. (SDM Vol. 3D, Appendix B.3.4)
#[derive(Clone, Copy, Debug)]
#[expect(
    non_camel_case_types,
    reason = "VMX names follow Intel SDM terminology."
)]
pub(super) enum VmcsHost32 {
    /// Host IA32_SYSENTER_CS.
    IA32_SYSENTER_CS = 0x4C00,
}
vmcs_write!(VmcsHost32, u32);

/// Natural-Width Host-State Fields. (SDM Vol. 3D, Appendix B.4.4)
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::upper_case_acronyms,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
#[cfg_attr(
    not(ktest),
    expect(
        dead_code,
        reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
    )
)]
pub(super) enum VmcsHostNW {
    /// Host CR0.
    CR0 = 0x6C00,
    /// Host CR3.
    CR3 = 0x6C02,
    /// Host CR4.
    CR4 = 0x6C04,
    /// Host FS base.
    FS_BASE = 0x6C06,
    /// Host GS base.
    GS_BASE = 0x6C08,
    /// Host TR base.
    TR_BASE = 0x6C0A,
    /// Host GDTR base.
    GDTR_BASE = 0x6C0C,
    /// Host IDTR base.
    IDTR_BASE = 0x6C0E,
    /// Host IA32_SYSENTER_ESP.
    IA32_SYSENTER_ESP = 0x6C10,
    /// Host IA32_SYSENTER_EIP.
    IA32_SYSENTER_EIP = 0x6C12,
    /// Host RSP.
    RSP = 0x6C14,
    /// Host RIP.
    RIP = 0x6C16,
}
#[cfg(ktest)]
vmcs_read!(VmcsHostNW, usize);
vmcs_write!(VmcsHostNW, usize);

/// 64-Bit Read-Only Data Fields. (SDM Vol. 3D, Appendix B.2.2)
#[derive(Clone, Copy, Debug)]
#[expect(
    non_camel_case_types,
    reason = "VMX names follow Intel SDM terminology."
)]
pub(crate) enum VmcsReadOnly64 {
    /// Guest-physical address.
    GUEST_PHYSICAL_ADDR = 0x2400,
}
vmcs_read!(VmcsReadOnly64, u64);

/// 32-Bit Read-Only Data Fields. (SDM Vol. 3D, Appendix B.3.2)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsReadOnly32 {
    /// VM-instruction error.
    VM_INSTRUCTION_ERROR = 0x4400,
    /// Exit reason.
    EXIT_REASON = 0x4402,
    /// VM-exit interruption information.
    VMEXIT_INTERRUPTION_INFO = 0x4404,
    /// VM-exit interruption error code.
    VMEXIT_INTERRUPTION_ERR_CODE = 0x4406,
    /// IDT-vectoring information field.
    IDT_VECTORING_INFO = 0x4408,
    /// IDT-vectoring error code.
    IDT_VECTORING_ERR_CODE = 0x440A,
    /// VM-exit instruction length.
    VMEXIT_INSTRUCTION_LEN = 0x440C,
    /// VM-exit instruction information.
    VMEXIT_INSTRUCTION_INFO = 0x440E,
}
vmcs_read!(VmcsReadOnly32, u32);

/// Natural-Width Read-Only Data Fields. (SDM Vol. 3D, Appendix B.4.2)
#[derive(Clone, Copy, Debug)]
#[expect(
    dead_code,
    non_camel_case_types,
    reason = "VMCS field names follow Intel SDM terminology, including fields reserved for future use."
)]
pub(crate) enum VmcsReadOnlyNW {
    /// Exit qualification.
    EXIT_QUALIFICATION = 0x6400,
    /// I/O RCX.
    IO_RCX = 0x6402,
    /// I/O RSI.
    IO_RSI = 0x6404,
    /// I/O RDI.
    IO_RDI = 0x6406,
    /// I/O RIP.
    IO_RIP = 0x6408,
    /// Guest-linear address.
    GUEST_LINEAR_ADDR = 0x640A,
}
vmcs_read!(VmcsReadOnlyNW, usize);

/// Writes a VMCS control field using the fixed-0/fixed-1 capability MSR.
/// Reference: Intel SDM Vol. 3D, A.2, A.3.
pub(super) fn set_control(
    control: VmcsControl32,
    capability_msr: Msr,
    old_value: u32,
    set: u32,
    clear: u32,
) -> Result<()> {
    let cap = capability_msr.read();
    let allowed0 = cap as u32;
    let allowed1 = (cap >> 32) as u32;
    assert_eq!(allowed0 & allowed1, allowed0);

    if (set & clear) != 0 {
        return Err(Error::InvalidArgs);
    }
    if (allowed1 & set) != set {
        // failed if set 0-bits in allowed1
        return Err(Error::InvalidArgs);
    }
    if (allowed0 & clear) != 0 {
        // failed if clear 1-bits in allowed0
        return Err(Error::InvalidArgs);
    }

    let flexible = !allowed0 & allowed1; // therse bits can be either 0 or 1
    let unknown = flexible & !(set | clear); // hypervisor untouched bits
    let default = unknown & old_value; // these bits keep unchanged in old value
    let fixed1 = allowed0; // these bits are fixed to 1
    control.write(fixed1 | default | set)?;
    Ok(())
}

macro_rules! def_exit_reasons {
    (
        $( #[$meta:meta] )*
        pub enum $name:ident {
            $( $variant:ident = $val:expr ),* $(,)?
        }
    ) => {
        $( #[$meta] )*
        #[repr(u32)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $( $variant = $val ),*
        }

        impl core::convert::TryFrom<u32> for $name {
            type Error = u32;

            fn try_from(value: u32) -> core::result::Result<Self, Self::Error> {
                match value {
                    $( $val => Ok($name::$variant), )*
                    _ => Err(value),
                }
            }
        }
    };
}

def_exit_reasons! {
    #[expect(
        clippy::upper_case_acronyms,
        non_camel_case_types,
        reason = "VMX names follow Intel SDM terminology."
    )]
    /// VMX basic exit reasons. (SDM Vol. 3D, Appendix C)
    pub enum VmxExitReason {
        EXCEPTION_NMI = 0,
        EXTERNAL_INTERRUPT = 1,
        TRIPLE_FAULT = 2,
        INIT = 3,
        SIPI = 4,
        SMI = 5,
        OTHER_SMI = 6,
        INTERRUPT_WINDOW = 7,
        NMI_WINDOW = 8,
        TASK_SWITCH = 9,
        CPUID = 10,
        GETSEC = 11,
        HLT = 12,
        INVD = 13,
        INVLPG = 14,
        RDPMC = 15,
        RDTSC = 16,
        RSM = 17,
        VMCALL = 18,
        VMCLEAR = 19,
        VMLAUNCH = 20,
        VMPTRLD = 21,
        VMPTRST = 22,
        VMREAD = 23,
        VMRESUME = 24,
        VMWRITE = 25,
        VMOFF = 26,
        VMON = 27,
        CR_ACCESS = 28,
        DR_ACCESS = 29,
        IO_INSTRUCTION = 30,
        MSR_READ = 31,
        MSR_WRITE = 32,
        INVALID_GUEST_STATE = 33,
        MSR_LOAD_FAIL = 34,
        MWAIT_INSTRUCTION = 36,
        MONITOR_TRAP_FLAG = 37,
        MONITOR_INSTRUCTION = 39,
        PAUSE_INSTRUCTION = 40,
        MCE_DURING_VMENTRY = 41,
        TPR_BELOW_THRESHOLD = 43,
        APIC_ACCESS = 44,
        VIRTUALIZED_EOI = 45,
        GDTR_IDTR = 46,
        LDTR_TR = 47,
        EPT_VIOLATION = 48,
        EPT_MISCONFIG = 49,
        INVEPT = 50,
        RDTSCP = 51,
        PREEMPTION_TIMER = 52,
        INVVPID = 53,
        WBINVD = 54,
        XSETBV = 55,
        APIC_WRITE = 56,
        RDRAND = 57,
        INVPCID = 58,
        VMFUNC = 59,
        ENCLS = 60,
        RDSEED = 61,
        PML_FULL = 62,
        XSAVES = 63,
        XRSTORS = 64,
        PCONFIG = 65,
        SPP_EVENT = 66,
        UMWAIT = 67,
        TPAUSE = 68,
        LOADIWKEY = 69,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VmxExitInfo {
    pub(crate) entry_failure: bool,
    pub(crate) exit_reason: u32,
    pub(crate) instruction_len: u32,
    pub(crate) exit_qualification: u64,
    pub(crate) guest_phys_addr: Paddr,
    pub(crate) guest_rip: Paddr,
}

/// Reads the VM-exit information fields from the current VMCS.
pub(crate) fn exit_info() -> Result<VmxExitInfo> {
    let reason_raw = VmcsReadOnly32::EXIT_REASON.read()?;
    let entry_failure = (reason_raw & (1 << 31)) != 0;
    let exit_reason = reason_raw & 0x7FFF_FFFF;
    let instruction_len = VmcsReadOnly32::VMEXIT_INSTRUCTION_LEN.read().unwrap_or(0);
    let exit_qualification = VmcsReadOnlyNW::EXIT_QUALIFICATION.read()? as _;
    let guest_phys_addr = VmcsReadOnly64::GUEST_PHYSICAL_ADDR.read()?;
    let guest_rip = VmcsGuestNW::RIP.read()?;
    Ok(VmxExitInfo {
        entry_failure,
        exit_reason,
        instruction_len,
        exit_qualification,
        guest_phys_addr: guest_phys_addr as _,
        guest_rip: guest_rip as _,
    })
}

/// Clears a VMCS region from any CPU that currently owns it.
#[inline]
pub(super) fn vmclear(vmcs: u64) -> Result<()> {
    let mut rflags: u64;
    unsafe {
        core::arch::asm!(
            "vmclear [{}]",
            "pushfq",
            "pop {}",
            in(reg) &vmcs,
            out(reg) rflags,
            options(nostack)
        );
    }

    if (rflags & 1) != 0 {
        return Err(Error::InvalidArgs);
    }
    if (rflags & (1 << 6)) != 0 {
        return Err(Error::InvalidArgs);
    }

    Ok(())
}

/// Makes a VMCS region current on this CPU.
#[inline]
pub(super) fn vmptrld(vmcs: u64) -> Result<()> {
    let mut rflags: u64;
    unsafe {
        core::arch::asm!(
            "vmptrld [{}]",
            "pushfq",
            "pop {}",
            in(reg) &vmcs,
            out(reg) rflags,
            options(nostack)
        );
    }

    if (rflags & 1) != 0 {
        return Err(Error::InvalidArgs);
    }
    if (rflags & (1 << 6)) != 0 {
        return Err(Error::InvalidArgs);
    }

    Ok(())
}
