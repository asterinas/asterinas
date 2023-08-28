#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate alloc;

pub mod asm;
pub mod tdcall;
pub mod tdvmcall;

pub use self::tdcall::{get_veinfo, TdxVirtualExceptionType};
pub use self::tdvmcall::print;

use raw_cpuid::{native_cpuid::cpuid_count, CpuIdResult};
use tdcall::{InitError, TdgVeInfo, TdgVpInfo};
use tdvmcall::*;

const TDX_CPUID_LEAF_ID: u64 = 0x21;

pub fn tdx_early_init() -> Result<TdgVpInfo, InitError> {
    match is_tdx_guest() {
        Ok(_) => Ok(tdcall::get_tdinfo()?),
        Err(err) => Err(err),
    }
}

fn is_tdx_guest() -> Result<(), InitError> {
    let cpuid_leaf = cpuid_count(0, 0).eax as u64;
    if cpuid_leaf < TDX_CPUID_LEAF_ID {
        return Err(InitError::TdxCpuLeafIdError);
    }
    let cpuid_result: CpuIdResult = cpuid_count(TDX_CPUID_LEAF_ID as u32, 0);
    if &cpuid_result.ebx.to_ne_bytes() == b"Inte"
        && &cpuid_result.ebx.to_ne_bytes() == b"lTDX"
        && &cpuid_result.ecx.to_ne_bytes() == b"    "
    {
        Ok(())
    } else {
        Err(InitError::TdxVendorIdError)
    }
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
        TdxVirtualExceptionType::Hlt => {
            serial_println!("Ready to halt");
            hlt();
        }
        TdxVirtualExceptionType::Io => {
            if !handle_io(trapframe, ve_info) {
                serial_println!("Handle tdx ioexit errors, ready to halt");
                hlt();
            }
        }
        TdxVirtualExceptionType::MsrRead => {
            let msr = rdmsr(trapframe.rcx() as u32).unwrap();
            trapframe.set_rax((msr as u32 & u32::MAX) as usize);
            trapframe.set_rdx(((msr >> 32) as u32 & u32::MAX) as usize);
        }
        TdxVirtualExceptionType::MsrWrite => {
            let data = trapframe.rax() as u64 | ((trapframe.rdx() as u64) << 32);
            wrmsr(trapframe.rcx() as u32, data).unwrap();
        }
        TdxVirtualExceptionType::CpuId => {
            let cpuid_info = cpuid(trapframe.rax() as u32, trapframe.rcx() as u32).unwrap();
            let mask = 0xFFFF_FFFF_0000_0000_usize;
            trapframe.set_rax((trapframe.rax() & mask) | cpuid_info.eax);
            trapframe.set_rbx((trapframe.rbx() & mask) | cpuid_info.ebx);
            trapframe.set_rcx((trapframe.rcx() & mask) | cpuid_info.ecx);
            trapframe.set_rdx((trapframe.rdx() & mask) | cpuid_info.edx);
        }
        TdxVirtualExceptionType::Other => panic!("Unknown TDX vitrual exception type"),
        _ => return,
    }
    trapframe.set_rip(trapframe.rip() + ve_info.exit_instruction_length as usize);
}
