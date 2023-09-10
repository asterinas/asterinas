use tdx_guest::{
    tdcall::TdgVeInfo,
    tdvmcall::{cpuid, hlt, rdmsr, wrmsr, IoSize},
    {serial_println, tdcall, tdvmcall, TdxVirtualExceptionType},
};
use trapframe::{GeneralRegs, TrapFrame};

pub struct TdxTrapFrame {
    rax: usize,
    rbx: usize,
    rcx: usize,
    rdx: usize,
    rsi: usize,
    rdi: usize,
    rip: usize,
}

impl From<TrapFrame> for TdxTrapFrame {
    fn from(tf: TrapFrame) -> Self {
        Self {
            rax: tf.rax,
            rbx: tf.rbx,
            rcx: tf.rcx,
            rdx: tf.rdx,
            rsi: tf.rsi,
            rdi: tf.rdi,
            rip: tf.rip,
        }
    }
}

impl From<GeneralRegs> for TdxTrapFrame {
    fn from(gr: GeneralRegs) -> Self {
        Self {
            rax: gr.rax,
            rbx: gr.rbx,
            rcx: gr.rcx,
            rdx: gr.rdx,
            rsi: gr.rsi,
            rdi: gr.rdi,
            rip: gr.rip,
        }
    }
}

pub fn handle_virtual_exception(trapframe: &mut TdxTrapFrame, ve_info: &TdgVeInfo) {
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
            let msr = unsafe { rdmsr(trapframe.rcx as u32).unwrap() };
            trapframe.rax = (msr as u32 & u32::MAX) as usize;
            trapframe.rdx = ((msr >> 32) as u32 & u32::MAX) as usize;
        }
        TdxVirtualExceptionType::MsrWrite => {
            let data = trapframe.rax as u64 | ((trapframe.rdx as u64) << 32);
            unsafe { wrmsr(trapframe.rcx as u32, data).unwrap() };
        }
        TdxVirtualExceptionType::CpuId => {
            let cpuid_info = cpuid(trapframe.rax as u32, trapframe.rcx as u32).unwrap();
            let mask = 0xFFFF_FFFF_0000_0000_usize;
            trapframe.rax = (trapframe.rax & mask) | cpuid_info.eax;
            trapframe.rbx = (trapframe.rbx & mask) | cpuid_info.ebx;
            trapframe.rcx = (trapframe.rcx & mask) | cpuid_info.ecx;
            trapframe.rdx = (trapframe.rdx & mask) | cpuid_info.edx;
        }
        TdxVirtualExceptionType::Other => panic!("Unknown TDX vitrual exception type"),
        _ => return,
    }
    trapframe.rip += ve_info.exit_instruction_length as usize;
}

fn handle_io(trapframe: &mut TdxTrapFrame, ve_info: &tdcall::TdgVeInfo) -> bool {
    let size = match ve_info.exit_qualification & 0x3 {
        0 => IoSize::Size1,
        1 => IoSize::Size2,
        3 => IoSize::Size4,
        _ => panic!("Invalid size value"),
    };
    let direction = if (ve_info.exit_qualification >> 3) & 0x1 == 0 {
        tdvmcall::Direction::Out
    } else {
        tdvmcall::Direction::In
    };
    let operand = if (ve_info.exit_qualification >> 6) & 0x1 == 0 {
        tdvmcall::Operand::Dx
    } else {
        tdvmcall::Operand::Immediate
    };
    let port = (ve_info.exit_qualification >> 16) as u16;

    match direction {
        tdvmcall::Direction::In => {
            trapframe.rax = tdvmcall::io_read(size, port).unwrap() as usize;
        }
        tdvmcall::Direction::Out => {
            tdvmcall::io_write(size, port, trapframe.rax as u32).unwrap();
        }
    };
    true
}
