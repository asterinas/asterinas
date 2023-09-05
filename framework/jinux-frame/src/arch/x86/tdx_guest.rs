use tdx_guest::{
    tdcall::TdgVeInfo,
    tdvmcall::{cpuid, hlt, rdmsr, wrmsr},
    {serial_println, tdcall, tdvmcall, TdxVirtualExceptionType},
};

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

fn io_handler(trapframe: &mut dyn TdxTrapFrame, ve_info: &tdcall::TdgVeInfo) -> bool {
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
            trapframe.set_rax(tdvmcall::io_read(size, port).unwrap() as usize);
        }
        tdvmcall::Direction::Out => {
            tdvmcall::io_write(size, port, trapframe.rax() as u32).unwrap();
        }
    };
    true
}

pub fn virtual_exception_handler(trapframe: &mut impl TdxTrapFrame, ve_info: &TdgVeInfo) {
    match ve_info.exit_reason.into() {
        TdxVirtualExceptionType::Hlt => {
            serial_println!("Ready to halt");
            hlt();
        }
        TdxVirtualExceptionType::Io => {
            if !io_handler(trapframe, ve_info) {
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
