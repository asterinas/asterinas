// SPDX-License-Identifier: BSD-3-Clause
// Copyright(c) 2023-2024 Intel Corporation.

//! The TDCALL instruction causes a VM exit to the Intel TDX module.
//! It is used to call guest-side Intel TDX functions. For more information about
//! TDCALL, please refer to the [Intel® TDX Module v1.5 ABI Specification](https://cdrdv2.intel.com/v1/dl/getContent/733579)

use crate::asm::asm_td_call;
use bitflags::bitflags;
use core::fmt;

/// TDCALL Instruction Leaf Numbers Definition.
#[repr(u64)]
pub enum TdcallNum {
    VpInfo = 1,
    MrRtmrExtend = 2,
    VpVeinfoGet = 3,
    MrReport = 4,
    VpCpuidveSet = 5,
    MemPageAccept = 6,
    VmRd = 7,
    VmWr = 8,
    MrVerifyreport = 22,
    MemPageAttrRd = 23,
    MemPageAttrWr = 24,
}

bitflags! {
    /// GuestTdAttributes is defined as a 64b field that specifies various guest TD attributes.
    /// It is reported to the guest TD by TDG.VP.INFO and as part of TDREPORT_STRUCT returned by TDG.MR.REPORT.
    pub struct GuestTdAttributes: u64 {
        /// Guest TD runs in off-TD debug mode.
        /// Its VCPU state and private memory are accessible by the host VMM.
        const DEBUG = 1 << 0;
        /// TD is migratable (using a Migration TD).
        const MIGRATABLE = 1 << 29;
        /// TD is allowed to use Supervisor Protection Keys.
        const PKS = 1 << 30;
        /// TD is allowed to use Key Locker. Must be 0.
        const KL = 1 << 31;
        /// TD is allowed to use Perfmon and PERF_METRICS capabilities.
        const PERFMON = 1 << 63;
    }
}

bitflags! {
    /// Controls whether CPUID executed by the guest TD will cause #VE unconditionally.
    struct CpuidveFlag: u64 {
        /// Flags that when CPL is 0, a CPUID executed
        /// by the guest TD will cause a #VE unconditionally.
        const SUPERVISOR = 1 << 0;
        /// Flags that when CPL > 0, a CPUID executed
        /// by the guest TD will cause a #VE unconditionally.
        const USER = 1 << 1;
    }
}

bitflags! {
    /// GPA Attributes (Single VM) Definition.
    pub struct GpaAttr: u16 {
        /// Read.
        const R = 1;
        /// Write.
        const W = 1 << 1;
        /// Execute (Supervisor).
        const XS = 1 << 2;
        /// Execute (User).
        const XU = 1 << 3;
        /// Verify Guest Paging.
        const VGP = 1 << 4;
        /// Paging-Write Access.
        const PWA = 1 << 5;
        /// Supervisor Shadow Stack.
        const SSS = 1 << 6;
        /// Suppress #VE.
        const SVE = 1 << 7;
        /// Indicates that the other bits are valid.
        /// If its value is 0, other fields are reserved and must be 0.
        const VALID = 1 << 15;
    }
}
pub struct PageAttr {
    /// Actual GPA mapping of the page.
    gpa_mapping: u64,
    /// Guest-visible page attributes.
    gpa_attr: GpaAttrAll,
}

/// GPA Attributes (all VMs) Definition.
pub struct GpaAttrAll {
    /// L1 GPA attributes.
    l1_attr: GpaAttr,
    /// GPA attributes for L2 VM #1.
    vm1_attr: GpaAttr,
    /// GPA attributes for L2 VM #2.
    vm2_attr: GpaAttr,
    /// GPA attributes for L2 VM #3.
    vm3_attr: GpaAttr,
}

impl From<u64> for GpaAttrAll {
    fn from(val: u64) -> Self {
        GpaAttrAll {
            l1_attr: GpaAttr::from_bits_truncate((val & 0xFFFF) as u16),
            vm1_attr: GpaAttr::from_bits_truncate(((val >> 16) & 0xFFFF) as u16),
            vm2_attr: GpaAttr::from_bits_truncate(((val >> 32) & 0xFFFF) as u16),
            vm3_attr: GpaAttr::from_bits_truncate(((val >> 48) & 0xFFFF) as u16),
        }
    }
}

impl From<GpaAttrAll> for u64 {
    fn from(s: GpaAttrAll) -> Self {
        let field1 = s.l1_attr.bits() as u64;
        let field2 = (s.vm1_attr.bits() as u64) << 16;
        let field3 = (s.vm2_attr.bits() as u64) << 32;
        let field4 = (s.vm3_attr.bits() as u64) << 48;
        field4 | field3 | field2 | field1
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct TdReport {
    /// REPORTMACSTRUCT for the TDG.MR.REPORT.
    pub report_mac: ReportMac,
    /// Additional attestable elements in the TD’s TCB are not reflected in the
    /// REPORTMACSTRUCT.CPUSVN – includes the Intel TDX module measurements.
    pub tee_tcb_info: [u8; 239],
    pub reserved: [u8; 17],
    /// TD’s attestable properties.
    pub tdinfo: TdInfo,
}

#[repr(C)]
#[derive(Debug)]
pub struct ReportMac {
    /// Type Header Structure.
    pub report_type: ReportType,
    pub cpu_svn: [u8; 16],
    /// SHA384 of TEE_TCB_INFO for TEEs implemented using Intel TDX.
    pub tee_tcb_info_hash: [u8; 48],
    /// SHA384 of TEE_INFO: a TEE-specific info structure (TDINFO_STRUCT or SGXINFO)
    /// or 0 if no TEE is represented.
    pub tee_info_hash: [u8; 48],
    /// A set of data used for communication between the caller and the target.
    pub report_data: [u8; 64],
    pub reserved: [u8; 32],
    /// The MAC over the REPORTMACSTRUCT with model-specific MAC.
    pub mac: [u8; 32],
}

#[derive(Debug)]
pub enum TeeType {
    SGX,
    TDX,
}

/// REPORTTYPE indicates the reported Trusted Execution Environment (TEE) type,
/// sub-type and version.
#[repr(C)]
#[derive(Debug)]
pub struct ReportType {
    /// Trusted Execution Environment (TEE) Type. 0x00: SGX, 0x81: TDX.
    pub tee_type: TeeType,
    /// TYPE-specific subtype.
    pub sub_type: u8,
    /// TYPE-specific version.
    pub version: u8,
    pub reserved: u8,
}

/// TDINFO_STRUCT is defined as the TDX-specific TEE_INFO part of TDG.MR.REPORT.
/// It contains the measurements and initial configuration of the TD that was
/// locked at initialization and a set of measurement registers that are run-time
/// extendable. These values are copied from the TDCS by the TDG.MR.REPORT function.
/// Refer to the [TDX Module Base Spec] for additional details.
#[repr(C)]
#[derive(Debug)]
pub struct TdInfo {
    /// TD’s ATTRIBUTES.
    pub attributes: u64,
    /// TD’s XFAM.
    pub xfam: u64,
    /// Measurement of the initial contents of the TD.
    pub mrtd: [u8; 48],
    /// Software-defined ID for non-owner-defined configuration of the
    /// guest TD – e.g., run-time or OS configuration.
    pub mr_config_id: [u8; 48],
    /// Software-defined ID for the guest TD’s owner.
    pub mr_owner: [u8; 48],
    /// Software-defined ID for owner-defined configuration of the
    /// guest TD – e.g., specific to the workload rather than the run-time or OS.
    pub mr_owner_config: [u8; 48],
    /// Array of NUM_RTMRS (4) run-time extendable measurement registers.
    pub rtmr0: [u8; 48],
    pub rtmr1: [u8; 48],
    pub rtmr2: [u8; 48],
    pub rtmr3: [u8; 48],
    /// If is one or more bound or pre-bound service TDs, SERVTD_HASH is the SHA384 hash of the
    /// TDINFO_STRUCTs of those service TDs bound. Else, SERVTD_HASH is 0.
    pub servtd_hash: [u8; 48],
    pub reserved: [u8; 64],
}

#[repr(C)]
#[derive(Debug)]
pub struct TdgVeInfo {
    pub exit_reason: u32,
    /// the 64-bit value that would have been saved into the VMCS as an exit qualification
    /// if a legacy VM exit had occurred instead of the virtualization exception.
    pub exit_qualification: u64,
    /// the 64-bit value that would have been saved into the VMCS as a guestlinear address
    /// if a legacy VM exit had occurred instead of the virtualization exception.
    pub guest_linear_address: u64,
    /// the 64-bit value that would have been saved into the VMCS as a guestphysical address
    /// if a legacy VM exit had occurred instead of the virtualization exception.
    pub guest_physical_address: u64,
    /// The 32-bit value that would have been saved into the VMCS as VM-exit instruction
    /// length if a legacy VM exit had occurred instead of the virtualization exception.
    pub exit_instruction_length: u32,
    /// The 32-bit value that would have been saved into the VMCS as VM-exit instruction
    /// information if a legacy VM exit had occurred instead of the virtualization exception.
    pub exit_instruction_info: u32,
}

#[derive(Debug)]
pub enum Gpaw {
    Bit48,
    Bit52,
}

impl From<u64> for Gpaw {
    fn from(val: u64) -> Self {
        match val {
            48 => Self::Bit48,
            52 => Self::Bit52,
            _ => panic!("Invalid gpaw"),
        }
    }
}

impl From<Gpaw> for u64 {
    fn from(s: Gpaw) -> Self {
        match s {
            Gpaw::Bit48 => 48,
            Gpaw::Bit52 => 52,
        }
    }
}

impl fmt::Display for Gpaw {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Gpaw::Bit48 => write!(f, "48-bit"),
            Gpaw::Bit52 => write!(f, "52-bit"),
        }
    }
}

#[derive(Debug)]
pub struct TdgVpInfo {
    /// The effective GPA width (in bits) for this TD (do not confuse with MAXPA).
    /// SHARED bit is at GPA bit GPAW-1.
    ///
    /// Only GPAW values 48 and 52 are possible.
    pub gpaw: Gpaw,
    /// The TD's ATTRIBUTES (provided as input to TDH.MNG.INIT)
    pub attributes: GuestTdAttributes,
    pub num_vcpus: u32,
    pub max_vcpus: u32,
    pub vcpu_index: u32,
    /// Indicates that the TDG.SYS.RD/RDM/RDCALL function are avaliable.
    pub sys_rd: u32,
}

#[derive(Debug, PartialEq)]
pub enum TdCallError {
    /// There is no valid #VE information.
    TdxNoValidVeInfo,
    /// Operand is invalid.
    TdxOperandInvalid,
    /// The operand is busy (e.g., it is locked in Exclusive mode).
    TdxOperandBusy,
    /// Page has already been accepted.
    TdxPageAlreadyAccepted,
    /// Requested page size does not match the current GPA mapping size.
    TdxPageSizeMismatch,
    Other,
}

impl From<u64> for TdCallError {
    fn from(val: u64) -> Self {
        match val {
            0xC000_0704 => Self::TdxNoValidVeInfo,
            0xC000_0100 => Self::TdxOperandInvalid,
            0x8000_0200 => Self::TdxOperandBusy,
            0x0000_0B0A => Self::TdxPageAlreadyAccepted,
            0xC000_0B0B => Self::TdxPageSizeMismatch,
            _ => Self::Other,
        }
    }
}

#[repr(C)]
#[derive(Default)]
pub(crate) struct TdcallArgs {
    rax: u64,
    rcx: u64,
    rdx: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
}

pub enum TdxVirtualExceptionType {
    Hlt,
    Io,
    MsrRead,
    MsrWrite,
    CpuId,
    VmCall,
    Mwait,
    Monitor,
    Wbinvd,
    Rdpmc,
    Other,
}

impl From<u32> for TdxVirtualExceptionType {
    fn from(val: u32) -> Self {
        match val {
            10 => Self::CpuId,
            12 => Self::Hlt,
            15 => Self::Rdpmc,
            18 => Self::VmCall,
            30 => Self::Io,
            31 => Self::MsrRead,
            32 => Self::MsrWrite,
            36 => Self::Mwait,
            39 => Self::Monitor,
            54 => Self::Wbinvd,
            _ => Self::Other,
        }
    }
}

#[derive(Debug)]
pub enum InitError {
    TdxVendorIdError,
    TdxCpuLeafIdError,
    TdxGetVpInfoError(TdCallError),
}

impl From<TdCallError> for InitError {
    fn from(error: TdCallError) -> Self {
        InitError::TdxGetVpInfoError(error)
    }
}

/// Get guest TD execution environment information.
pub fn get_tdinfo() -> Result<TdgVpInfo, TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::VpInfo as u64,
        ..Default::default()
    };
    td_call(&mut args)?;
    let td_info = TdgVpInfo {
        gpaw: Gpaw::from(args.rcx),
        attributes: GuestTdAttributes::from_bits_truncate(args.rdx),
        num_vcpus: args.r8 as u32,
        max_vcpus: (args.r8 >> 32) as u32,
        vcpu_index: args.r9 as u32,
        sys_rd: args.r10 as u32,
    };
    Ok(td_info)
}

/// Get Virtualization Exception Information for the recent #VE exception.
pub fn get_veinfo() -> Result<TdgVeInfo, TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::VpVeinfoGet as u64,
        ..Default::default()
    };
    td_call(&mut args)?;
    let ve_info = TdgVeInfo {
        exit_reason: args.rcx as u32,
        exit_qualification: args.rdx,
        guest_linear_address: args.r8,
        guest_physical_address: args.r9,
        exit_instruction_length: args.r10 as u32,
        exit_instruction_info: (args.r10 >> 32) as u32,
    };
    Ok(ve_info)
}

/// Extend a TDCS.RTMR measurement register.
pub fn extend_rtmr() -> Result<(), TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::MrRtmrExtend as u64,
        ..Default::default()
    };
    td_call(&mut args)
}

/// TDG.MR.REPORT creates a TDREPORT_STRUCT structure that contains the measurements/configuration
/// information of the guest TD that called the function, measurements/configuration information
/// of the Intel TDX module and a REPORTMACSTRUCT.
pub fn get_report(report_gpa: &[u8], data_gpa: &[u8]) -> Result<(), TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::MrReport as u64,
        rcx: report_gpa.as_ptr() as u64,
        rdx: data_gpa.as_ptr() as u64,
        ..Default::default()
    };
    td_call(&mut args)
}

/// Verify a cryptographic REPORTMACSTRUCT that describes the contents of a TD,
/// to determine that it was created on the current TEE on the current platform.
pub fn verify_report(report_mac_gpa: &[u8]) -> Result<(), TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::MrVerifyreport as u64,
        rcx: report_mac_gpa.as_ptr() as u64,
        ..Default::default()
    };
    td_call(&mut args)
}

/// Accept a pending private page and initialize it to all-0 using the TD ephemeral private key.
/// # Safety
/// The 'gpa' parameter must be a valid address.
pub unsafe fn accept_page(sept_level: u64, gpa: &[u8]) -> Result<(), TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::MemPageAccept as u64,
        rcx: sept_level | ((gpa.as_ptr() as u64) << 12),
        ..Default::default()
    };
    td_call(&mut args)
}

/// Read the GPA mapping and attributes of a TD private page.
pub fn read_page_attr(gpa: &[u8]) -> Result<PageAttr, TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::MemPageAttrRd as u64,
        rcx: gpa.as_ptr() as u64,
        ..Default::default()
    };
    td_call(&mut args)?;
    let page_attr = PageAttr {
        gpa_mapping: args.rcx,
        gpa_attr: GpaAttrAll::from(args.rdx),
    };
    Ok(page_attr)
}

/// Write the attributes of a private page. Create or remove L2 page aliases as required.
pub fn write_page_attr(page_attr: PageAttr, attr_flags: u64) -> Result<PageAttr, TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::MemPageAttrWr as u64,
        rcx: page_attr.gpa_mapping,
        rdx: u64::from(page_attr.gpa_attr),
        r8: attr_flags,
        ..Default::default()
    };
    td_call(&mut args)?;
    let page_attr = PageAttr {
        gpa_mapping: args.rcx,
        gpa_attr: GpaAttrAll::from(args.rdx),
    };
    Ok(page_attr)
}

/// Read a TD-scope metadata field (control structure field) of a TD.
pub fn read_td_metadata(field_identifier: u64) -> Result<u64, TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::VmRd as u64,
        rdx: field_identifier,
        ..Default::default()
    };
    td_call(&mut args)?;
    Ok(args.r8)
}

/// Write a TD-scope metadata field (control structure field) of a TD.
///
/// - data: data to write to the field
///
/// - write_mask: a 64b write mask to indicate which bits of the value
/// in R8 are to be written to the field.
///
/// It returns previous contents of the field.
pub fn write_td_metadata(
    field_identifier: u64,
    data: u64,
    write_mask: u64,
) -> Result<u64, TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::VmWr as u64,
        rdx: field_identifier,
        r8: data,
        r9: write_mask,
        ..Default::default()
    };
    td_call(&mut args).map(|_| args.r8)
}

/// TDG.VP.CPUIDVE.SET controls unconditional #VE on CPUID execution by the guest TD.
///
/// Note: TDG.VP.CPUIDVE.SET is provided for backward compatibility.
///
/// The guest TD may control the same settings by writing to the
/// VCPU-scope metadata fields CPUID_SUPERVISOR_VE and CPUID_USER_VE using TDG.VP.WR.
pub fn set_cpuidve(cpuidve_flag: u64) -> Result<(), TdCallError> {
    let mut args = TdcallArgs {
        rax: TdcallNum::VpCpuidveSet as u64,
        rcx: cpuidve_flag,
        ..Default::default()
    };
    td_call(&mut args)
}

fn td_call(args: &mut TdcallArgs) -> Result<(), TdCallError> {
    let result = unsafe { asm_td_call(args) };
    match result {
        0 => Ok(()),
        _ => Err(result.into()),
    }
}
