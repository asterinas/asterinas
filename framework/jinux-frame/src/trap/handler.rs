use crate::{arch::irq::IRQ_LIST, cpu::CpuException};

#[cfg(feature = "intel_tdx")]
use tdx_guest::{serial_println, tdcall, tdvmcall, TdxVirtualExceptionType};
use trapframe::TrapFrame;

#[cfg(feature = "intel_tdx")]
pub fn virtual_exception_handler(trapframe: &mut TrapFrame, ve_info: &tdcall::TdgVeInfo) {
    match ve_info.exit_reason.into() {
        TdxVirtualExceptionType::Hlt => {
            serial_println!("Ready to halt");
            tdvmcall::hlt();
        }
        TdxVirtualExceptionType::Io => {
            if !handle_ve_io(trapframe, ve_info) {
                serial_println!("Handle tdx ioexit errors, ready to halt");
                tdvmcall::hlt();
            }
        }
        TdxVirtualExceptionType::MsrRead => {
            let msr = tdvmcall::rdmsr(trapframe.rcx as u32).unwrap();
            trapframe.rax = (msr as u32 & u32::MAX) as usize;
            trapframe.rdx = ((msr >> 32) as u32 & u32::MAX) as usize;
        }
        TdxVirtualExceptionType::MsrWrite => {
            let data = trapframe.rax as u64 | ((trapframe.rdx as u64) << 32);
            tdvmcall::wrmsr(trapframe.rcx as u32, data).unwrap();
        }
        TdxVirtualExceptionType::CpuId => {
            let cpuid_info = tdvmcall::cpuid(trapframe.rax as u32, trapframe.rcx as u32).unwrap();
            let mask = 0xFFFF_FFFF_0000_0000_usize;
            trapframe.rax = (trapframe.rax & mask) | cpuid_info.eax;
            trapframe.rbx = (trapframe.rbx & mask) | cpuid_info.ebx;
            trapframe.rcx = (trapframe.rcx & mask) | cpuid_info.ecx;
            trapframe.rdx = (trapframe.rdx & mask) | cpuid_info.edx;
        }
        TdxVirtualExceptionType::Other => panic!("Unknown TDX vitrual exception type"),
        _ => return,
    }
    trapframe.rip = trapframe.rip + ve_info.exit_instruction_length as usize;
}

#[cfg(feature = "intel_tdx")]
pub fn handle_ve_io(trapframe: &mut TrapFrame, ve_info: &tdcall::TdgVeInfo) -> bool {
    let size = match ve_info.exit_qualification & 0x3 {
        0 => 1,
        1 => 2,
        3 => 4,
        _ => panic!("Invalid size value"),
    };
    let direction = if (ve_info.exit_qualification >> 3) & 0x1 == 0 {
        tdvmcall::Direction::Out
    } else {
        tdvmcall::Direction::In
    };
    let string = (ve_info.exit_qualification >> 4) & 0x1 == 1;
    let repeat = (ve_info.exit_qualification >> 5) & 0x1 == 1;
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

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if CpuException::is_cpu_exception(f.trap_num as u16) {
        #[cfg(feature = "intel_tdx")]
        if f.trap_num as u16 == 20 {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            virtual_exception_handler(f, &ve_info);
        }
        #[cfg(not(feature = "intel_tdx"))]
        panic!("cannot handle kernel cpu fault now, information:{:#x?}", f);
    } else {
        call_irq_callback_functions(f);
    }
}

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame) {
    let irq_line = IRQ_LIST.get().unwrap().get(trap_frame.trap_num).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    if !CpuException::is_cpu_exception(trap_frame.trap_num as u16) {
        crate::arch::interrupts_ack();
    }
}
