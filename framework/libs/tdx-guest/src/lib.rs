#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate alloc;

pub mod asm;
pub mod tdcall;
pub mod tdvmcall;

pub use self::tdcall::{deadloop, tdg_vp_veinfo_get, TdxVirtualExceptionType};
pub use self::tdvmcall::print;

use alloc::string::{String, ToString};
use raw_cpuid::{native_cpuid::cpuid_count, CpuIdResult};
use tdcall::{tdg_vp_vmcall, InitError, TdgVeInfo, TdgVpInfo};

const TDX_CPUID_LEAF_ID: u64 = 0x21;

pub fn tdx_early_init() -> Result<TdgVpInfo, InitError> {
    match is_tdx_guest() {
        Ok(_) => Ok(tdcall::tdg_vp_info()?),
        Err(err) => Err(err),
    }
}

fn is_tdx_guest() -> Result<(), InitError> {
    let cpuid_leaf = cpuid_count(0, 0).eax as u64;
    if cpuid_leaf < TDX_CPUID_LEAF_ID {
        return Err(InitError::TdxCpuLeafIdError);
    }
    let cpuid_result: CpuIdResult = cpuid_count(TDX_CPUID_LEAF_ID as u32, 0);
    if convert_ascii(cpuid_result.ebx) == "Inte"
        && convert_ascii(cpuid_result.edx) == "lTDX"
        && convert_ascii(cpuid_result.ecx) == "    "
    {
        Ok(())
    } else {
        Err(InitError::TdxVendorIdError)
    }
}

fn convert_ascii(reg: u32) -> String {
    let bytes = [
        (reg & 0xFF) as u8,
        ((reg >> 8) & 0xFF) as u8,
        ((reg >> 16) & 0xFF) as u8,
        ((reg >> 24) & 0xFF) as u8,
    ];
    String::from_utf8_lossy(&bytes).to_string()
}

pub trait TdxTrapFrame {
    fn rax(&self) -> usize;
    fn set_rax(&mut self, rax: usize);
    fn rbx(&self) -> usize;
    fn set_rbx(&mut self, rbx: usize);
    fn rcx(&self) -> usize;
    fn set_rcx(&mut self, rcx: usize);
    fn rdx(&self) -> usize;
    fn set_rdx(&mut self, rdx: usize);
    fn rsi(&self) -> usize;
    fn set_rsi(&mut self, rsi: usize);
    fn rdi(&self) -> usize;
    fn set_rdi(&mut self, rdi: usize);
    fn rip(&self) -> usize;
    fn set_rip(&mut self, rip: usize);
}

pub fn virtual_exception_handler(trapframe: &mut impl TdxTrapFrame, ve_info: &TdgVeInfo) {
    match ve_info.exit_reason.into() {
        TdxVirtualExceptionType::Hlt
        | TdxVirtualExceptionType::Io
        | TdxVirtualExceptionType::MsrRead
        | TdxVirtualExceptionType::MsrWrite
        | TdxVirtualExceptionType::CpuId => tdg_vp_vmcall(trapframe, ve_info),
        TdxVirtualExceptionType::Other => panic!("Unknown TDX vitrual exception type"),
        _ => return,
    };
    trapframe.set_rip(trapframe.rip() + ve_info.exit_instruction_length as usize);
}

