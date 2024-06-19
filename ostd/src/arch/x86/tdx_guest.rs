// SPDX-License-Identifier: MPL-2.0

use iced_x86::{Code, Decoder, DecoderOptions, Instruction, Register};
use log::warn;
use tdx_guest::{
    serial_println, tdcall,
    tdcall::{accept_page, TdgVeInfo},
    tdvmcall,
    tdvmcall::{cpuid, hlt, map_gpa, rdmsr, read_mmio, write_mmio, wrmsr, IoSize},
    TdxVirtualExceptionType,
};
use trapframe::TrapFrame;

use crate::{
    mm::{
        kspace::{BOOT_PAGE_TABLE, KERNEL_BASE_VADDR, KERNEL_END_VADDR, KERNEL_PAGE_TABLE},
        paddr_to_vaddr,
        page_prop::{PageProperty, PrivilegedPageFlags as PrivFlags},
        page_table::PageTableError,
        PAGE_SIZE,
    },
    prelude::Paddr,
};

const SHARED_BIT: u8 = 51;
const SHARED_MASK: u64 = 1u64 << SHARED_BIT;

// Intel TDX guest physical address. Maybe protected(private) gpa or unprotected(shared) gpa.
pub type TdxGpa = usize;

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
    fn r8(&self) -> usize;
    fn set_r8(&mut self, r8: usize);
    fn r9(&self) -> usize;
    fn set_r9(&mut self, r9: usize);
    fn r10(&self) -> usize;
    fn set_r10(&mut self, r10: usize);
    fn r11(&self) -> usize;
    fn set_r11(&mut self, r11: usize);
    fn r12(&self) -> usize;
    fn set_r12(&mut self, r12: usize);
    fn r13(&self) -> usize;
    fn set_r13(&mut self, r13: usize);
    fn r14(&self) -> usize;
    fn set_r14(&mut self, r14: usize);
    fn r15(&self) -> usize;
    fn set_r15(&mut self, r15: usize);
    fn rbp(&self) -> usize;
    fn set_rbp(&mut self, rbp: usize);
}

enum InstrMmioType {
    Write,
    WriteImm,
    Read,
    ReadZeroExtend,
    ReadSignExtend,
    Movs,
}

#[derive(Debug)]
enum MmioError {
    Unimplemented,
    InvalidInstruction,
    InvalidAddress,
    DecodeFailed,
    TdVmcallError(tdvmcall::TdVmcallError),
}

#[derive(Debug)]
pub enum PageConvertError {
    PageTableError(PageTableError),
    TdCallError(tdcall::TdCallError),
    TdVmcallError((u64, tdvmcall::TdVmcallError)),
}

pub fn handle_virtual_exception(trapframe: &mut dyn TdxTrapFrame, ve_info: &TdgVeInfo) {
    let mut instr_len = ve_info.exit_instruction_length;
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
            let msr = unsafe { rdmsr(trapframe.rcx() as u32).unwrap() };
            trapframe.set_rax((msr as u32 & u32::MAX) as usize);
            trapframe.set_rdx(((msr >> 32) as u32 & u32::MAX) as usize);
        }
        TdxVirtualExceptionType::MsrWrite => {
            let data = trapframe.rax() as u64 | ((trapframe.rdx() as u64) << 32);
            unsafe { wrmsr(trapframe.rcx() as u32, data).unwrap() };
        }
        TdxVirtualExceptionType::CpuId => {
            let cpuid_info = cpuid(trapframe.rax() as u32, trapframe.rcx() as u32).unwrap();
            let mask = 0xFFFF_FFFF_0000_0000_usize;
            trapframe.set_rax((trapframe.rax() & mask) | cpuid_info.eax);
            trapframe.set_rbx((trapframe.rbx() & mask) | cpuid_info.ebx);
            trapframe.set_rcx((trapframe.rcx() & mask) | cpuid_info.ecx);
            trapframe.set_rdx((trapframe.rdx() & mask) | cpuid_info.edx);
        }
        TdxVirtualExceptionType::EptViolation => {
            if is_protected_gpa(ve_info.guest_physical_address as TdxGpa) {
                serial_println!("Unexpected EPT-violation on private memory");
                hlt();
            }
            instr_len = handle_mmio(trapframe, ve_info).unwrap() as u32;
        }
        TdxVirtualExceptionType::Other => {
            serial_println!("Unknown TDX vitrual exception type");
            hlt();
        }
        _ => return,
    }
    trapframe.set_rip(trapframe.rip() + instr_len as usize);
}

fn handle_io(trapframe: &mut dyn TdxTrapFrame, ve_info: &tdcall::TdgVeInfo) -> bool {
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
    let _operand = if (ve_info.exit_qualification >> 6) & 0x1 == 0 {
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

fn is_protected_gpa(gpa: TdxGpa) -> bool {
    (gpa as u64 & SHARED_MASK) == 0
}

fn handle_mmio(trapframe: &mut dyn TdxTrapFrame, ve_info: &TdgVeInfo) -> Result<usize, MmioError> {
    // Get instruction
    let instr = decode_instr(trapframe.rip())?;

    // Decode MMIO instruction
    match decode_mmio(&instr) {
        Some((mmio, size)) => {
            match mmio {
                InstrMmioType::Write => {
                    let value = match instr.op1_register() {
                        Register::RCX => trapframe.rcx() as u64,
                        Register::ECX => (trapframe.rcx() & 0xFFFF_FFFF) as u64,
                        Register::CX => (trapframe.rcx() & 0xFFFF) as u64,
                        Register::CL => (trapframe.rcx() & 0xFF) as u64,
                        _ => todo!(),
                    };
                    // SAFETY: The mmio_gpa obtained from `ve_info` is valid, and the value and size parsed from the instruction are valid.
                    unsafe {
                        write_mmio(size, ve_info.guest_physical_address, value)
                            .map_err(MmioError::TdVmcallError)?
                    }
                }
                InstrMmioType::WriteImm => {
                    let value = instr.immediate(0);
                    // SAFETY: The mmio_gpa obtained from `ve_info` is valid, and the value and size parsed from the instruction are valid.
                    unsafe {
                        write_mmio(size, ve_info.guest_physical_address, value)
                            .map_err(MmioError::TdVmcallError)?
                    }
                }
                InstrMmioType::Read =>
                // SAFETY: The mmio_gpa obtained from `ve_info` is valid, and the size parsed from the instruction is valid.
                unsafe {
                    let read_res = read_mmio(size, ve_info.guest_physical_address)
                        .map_err(MmioError::TdVmcallError)?
                        as usize;
                    match instr.op0_register() {
                        Register::RAX => trapframe.set_rax(read_res),
                        Register::EAX => {
                            trapframe.set_rax((trapframe.rax() & 0xFFFF_FFFF_0000_0000) | read_res)
                        }
                        Register::AX => {
                            trapframe.set_rax((trapframe.rax() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::AL => {
                            trapframe.set_rax((trapframe.rax() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::RBX => trapframe.set_rbx(read_res),
                        Register::EBX => {
                            trapframe.set_rbx((trapframe.rbx() & 0xFFFF_FFFF_0000_0000) | read_res)
                        }
                        Register::BX => {
                            trapframe.set_rbx((trapframe.rbx() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::BL => {
                            trapframe.set_rbx((trapframe.rbx() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::RCX => trapframe.set_rcx(read_res),
                        Register::ECX => {
                            trapframe.set_rcx((trapframe.rcx() & 0xFFFF_FFFF_0000_0000) | read_res)
                        }
                        Register::CX => {
                            trapframe.set_rcx((trapframe.rcx() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::CL => {
                            trapframe.set_rcx((trapframe.rcx() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::RDX => trapframe.set_rdx(read_res),
                        Register::EDX => {
                            trapframe.set_rdx((trapframe.rdx() & 0xFFFF_FFFF_0000_0000) | read_res)
                        }
                        Register::DX => {
                            trapframe.set_rdx((trapframe.rdx() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::DL => {
                            trapframe.set_rdx((trapframe.rdx() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::SIL => {
                            trapframe.set_rsi((trapframe.rsi() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::DIL => {
                            trapframe.set_rdi((trapframe.rdi() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R8L => {
                            trapframe.set_r8((trapframe.r8() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R9L => {
                            trapframe.set_r9((trapframe.r9() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R10L => {
                            trapframe.set_r10((trapframe.r10() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R11L => {
                            trapframe.set_r11((trapframe.r11() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R11W => {
                            trapframe.set_r11((trapframe.r11() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::R12L => {
                            trapframe.set_r12((trapframe.r12() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R13L => {
                            trapframe.set_r13((trapframe.r13() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R13W => {
                            trapframe.set_r13((trapframe.r13() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::R14L => {
                            trapframe.set_r14((trapframe.r14() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::R14D => {
                            trapframe.set_r14((trapframe.r14() & 0xFFFF_FFFF_0000_0000) | read_res)
                        }
                        Register::R15L => {
                            trapframe.set_r15((trapframe.r15() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        Register::BP => {
                            trapframe.set_rbp((trapframe.rbp() & 0xFFFF_FFFF_FFFF_0000) | read_res)
                        }
                        Register::BPL => {
                            trapframe.set_rbp((trapframe.rbp() & 0xFFFF_FFFF_FFFF_FF00) | read_res)
                        }
                        _ => return Err(MmioError::Unimplemented),
                    }
                },
                InstrMmioType::ReadZeroExtend =>
                // SAFETY: The mmio_gpa obtained from `ve_info` is valid, and the size parsed from the instruction is valid.
                unsafe {
                    let read_res = read_mmio(size, ve_info.guest_physical_address)
                        .map_err(MmioError::TdVmcallError)?
                        as usize;
                    match instr.op0_register() {
                        Register::RAX | Register::EAX | Register::AX | Register::AL => {
                            trapframe.set_rax(read_res)
                        }
                        Register::RBX | Register::EBX | Register::BX | Register::BL => {
                            trapframe.set_rbx(read_res)
                        }
                        Register::RCX | Register::ECX | Register::CX | Register::CL => {
                            trapframe.set_rcx(read_res)
                        }
                        _ => return Err(MmioError::Unimplemented),
                    }
                },
                InstrMmioType::ReadSignExtend => return Err(MmioError::Unimplemented),
                // MMIO was accessed with an instruction that could not be decoded or handled properly.
                InstrMmioType::Movs => return Err(MmioError::InvalidInstruction),
            }
        }
        None => {
            return Err(MmioError::DecodeFailed);
        }
    }
    Ok(instr.len())
}

fn decode_instr(rip: usize) -> Result<Instruction, MmioError> {
    if !(KERNEL_BASE_VADDR..KERNEL_END_VADDR).contains(&rip) {
        return Err(MmioError::InvalidAddress);
    }
    let code_data = {
        const MAX_X86_INSTR_LEN: usize = 15;
        let mut data = [0u8; MAX_X86_INSTR_LEN];
        // SAFETY:
        // This is safe because we are ensuring that 'rip' is a valid kernel virtual address before this operation.
        // We are also ensuring that the size of the data we are copying does not exceed 'MAX_X86_INSTR_LEN'.
        // Therefore, we are not reading any memory that we shouldn't be, and we are not causing any undefined behavior.
        unsafe {
            core::ptr::copy_nonoverlapping(rip as *const u8, data.as_mut_ptr(), data.len());
        }
        data
    };
    let mut decoder = Decoder::with_ip(64, &code_data, rip as u64, DecoderOptions::NONE);
    let mut instr = Instruction::default();
    decoder.decode_out(&mut instr);
    if instr.is_invalid() {
        return Err(MmioError::InvalidInstruction);
    }
    Ok(instr)
}

fn decode_mmio(instr: &Instruction) -> Option<(InstrMmioType, IoSize)> {
    match instr.code() {
        // 0x88
        Code::Mov_rm8_r8 => Some((InstrMmioType::Write, IoSize::Size1)),
        // 0x89
        Code::Mov_rm16_r16 => Some((InstrMmioType::Write, IoSize::Size2)),
        Code::Mov_rm32_r32 => Some((InstrMmioType::Write, IoSize::Size4)),
        Code::Mov_rm64_r64 => Some((InstrMmioType::Write, IoSize::Size8)),
        // 0xc6
        Code::Mov_rm8_imm8 => Some((InstrMmioType::WriteImm, IoSize::Size1)),
        // 0xc7
        Code::Mov_rm16_imm16 => Some((InstrMmioType::WriteImm, IoSize::Size2)),
        Code::Mov_rm32_imm32 => Some((InstrMmioType::WriteImm, IoSize::Size4)),
        Code::Mov_rm64_imm32 => Some((InstrMmioType::WriteImm, IoSize::Size8)),
        // 0x8a
        Code::Mov_r8_rm8 => Some((InstrMmioType::Read, IoSize::Size1)),
        // 0x8b
        Code::Mov_r16_rm16 => Some((InstrMmioType::Read, IoSize::Size2)),
        Code::Mov_r32_rm32 => Some((InstrMmioType::Read, IoSize::Size4)),
        Code::Mov_r64_rm64 => Some((InstrMmioType::Read, IoSize::Size8)),
        // 0xa4
        Code::Movsb_m8_m8 => Some((InstrMmioType::Movs, IoSize::Size1)),
        // 0xa5
        Code::Movsw_m16_m16 => Some((InstrMmioType::Movs, IoSize::Size2)),
        Code::Movsd_m32_m32 => Some((InstrMmioType::Movs, IoSize::Size4)),
        Code::Movsq_m64_m64 => Some((InstrMmioType::Movs, IoSize::Size8)),
        // 0x0f 0xb6
        Code::Movzx_r16_rm8 | Code::Movzx_r32_rm8 | Code::Movzx_r64_rm8 => {
            Some((InstrMmioType::ReadZeroExtend, IoSize::Size1))
        }
        // 0x0f 0xb7
        Code::Movzx_r16_rm16 | Code::Movzx_r32_rm16 | Code::Movzx_r64_rm16 => {
            Some((InstrMmioType::ReadZeroExtend, IoSize::Size2))
        }
        // 0x0f 0xbe
        Code::Movsx_r16_rm8 | Code::Movsx_r32_rm8 | Code::Movsx_r64_rm8 => {
            Some((InstrMmioType::ReadSignExtend, IoSize::Size1))
        }
        // 0x0f 0xbf
        Code::Movsx_r16_rm16 | Code::Movsx_r32_rm16 | Code::Movsx_r64_rm16 => {
            Some((InstrMmioType::ReadSignExtend, IoSize::Size2))
        }
        _ => None,
    }
}

/// Sets the given physical address range to Intel TDX shared pages.
/// Clears the data within the given address range.
/// Make sure the provided physical address is page size aligned.
///
/// # Safety
///
/// To safely use this function, the caller must ensure that:
/// - The given guest physical address range is currently mapped in the page table.
/// - The `page_num` argument represents a valid number of pages.
/// - This function will erase any valid data in the range and should not assume that the data will still be there after the operation.
pub unsafe fn unprotect_gpa_range(gpa: Paddr, page_num: usize) -> Result<(), PageConvertError> {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    if gpa & PAGE_MASK != 0 {
        warn!("Misaligned address: {:x}", gpa);
    }
    // Protect the page in the kernel page table.
    let pt = KERNEL_PAGE_TABLE.get().unwrap();
    let protect_op = |prop: &mut PageProperty| {
        *prop = PageProperty {
            flags: prop.flags,
            cache: prop.cache,
            priv_flags: prop.priv_flags | PrivFlags::SHARED,
        }
    };
    let vaddr = paddr_to_vaddr(gpa);
    pt.protect(&(vaddr..vaddr + page_num * PAGE_SIZE), protect_op)
        .map_err(PageConvertError::PageTableError)?;
    // Protect the page in the boot page table if in the boot phase.
    {
        let mut boot_pt_lock = BOOT_PAGE_TABLE.lock();
        if let Some(boot_pt) = boot_pt_lock.as_mut() {
            for i in 0..page_num {
                let vaddr = paddr_to_vaddr(gpa + i * PAGE_SIZE);
                boot_pt.protect_base_page(vaddr, protect_op);
            }
        }
    }
    map_gpa(
        (gpa & (!PAGE_MASK)) as u64 | SHARED_MASK,
        (page_num * PAGE_SIZE) as u64,
    )
    .map_err(PageConvertError::TdVmcallError)
}

/// Sets the given physical address range to Intel TDX private pages.
/// Make sure the provided physical address is page size aligned.
///
/// # Safety
///
/// To safely use this function, the caller must ensure that:
/// - The given guest physical address range is currently mapped in the page table.
/// - The `page_num` argument represents a valid number of pages.
///
pub unsafe fn protect_gpa_range(gpa: Paddr, page_num: usize) -> Result<(), PageConvertError> {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    if gpa & !PAGE_MASK == 0 {
        warn!("Misaligned address: {:x}", gpa);
    }
    // Protect the page in the kernel page table.
    let pt = KERNEL_PAGE_TABLE.get().unwrap();
    let protect_op = |prop: &mut PageProperty| {
        *prop = PageProperty {
            flags: prop.flags,
            cache: prop.cache,
            priv_flags: prop.priv_flags - PrivFlags::SHARED,
        }
    };
    let vaddr = paddr_to_vaddr(gpa);
    pt.protect(&(vaddr..vaddr + page_num * PAGE_SIZE), protect_op)
        .map_err(PageConvertError::PageTableError)?;
    // Protect the page in the boot page table if in the boot phase.
    {
        let mut boot_pt_lock = BOOT_PAGE_TABLE.lock();
        if let Some(boot_pt) = boot_pt_lock.as_mut() {
            for i in 0..page_num {
                let vaddr = paddr_to_vaddr(gpa + i * PAGE_SIZE);
                boot_pt.protect_base_page(vaddr, protect_op);
            }
        }
    }
    map_gpa((gpa & PAGE_MASK) as u64, (page_num * PAGE_SIZE) as u64)
        .map_err(PageConvertError::TdVmcallError)?;
    for i in 0..page_num {
        unsafe {
            accept_page(0, (gpa + i * PAGE_SIZE) as u64).map_err(PageConvertError::TdCallError)?;
        }
    }
    Ok(())
}

#[cfg(feature = "intel_tdx")]
impl TdxTrapFrame for TrapFrame {
    fn rax(&self) -> usize {
        self.rax
    }
    fn set_rax(&mut self, rax: usize) {
        self.rax = rax;
    }
    fn rbx(&self) -> usize {
        self.rbx
    }
    fn set_rbx(&mut self, rbx: usize) {
        self.rbx = rbx;
    }
    fn rcx(&self) -> usize {
        self.rcx
    }
    fn set_rcx(&mut self, rcx: usize) {
        self.rcx = rcx;
    }
    fn rdx(&self) -> usize {
        self.rdx
    }
    fn set_rdx(&mut self, rdx: usize) {
        self.rdx = rdx;
    }
    fn rsi(&self) -> usize {
        self.rsi
    }
    fn set_rsi(&mut self, rsi: usize) {
        self.rsi = rsi;
    }
    fn rdi(&self) -> usize {
        self.rdi
    }
    fn set_rdi(&mut self, rdi: usize) {
        self.rdi = rdi;
    }
    fn rip(&self) -> usize {
        self.rip
    }
    fn set_rip(&mut self, rip: usize) {
        self.rip = rip;
    }
    fn r8(&self) -> usize {
        self.r8
    }
    fn set_r8(&mut self, r8: usize) {
        self.r8 = r8;
    }
    fn r9(&self) -> usize {
        self.r9
    }
    fn set_r9(&mut self, r9: usize) {
        self.r9 = r9;
    }
    fn r10(&self) -> usize {
        self.r10
    }
    fn set_r10(&mut self, r10: usize) {
        self.r10 = r10;
    }
    fn r11(&self) -> usize {
        self.r11
    }
    fn set_r11(&mut self, r11: usize) {
        self.r11 = r11;
    }
    fn r12(&self) -> usize {
        self.r12
    }
    fn set_r12(&mut self, r12: usize) {
        self.r12 = r12;
    }
    fn r13(&self) -> usize {
        self.r13
    }
    fn set_r13(&mut self, r13: usize) {
        self.r13 = r13;
    }
    fn r14(&self) -> usize {
        self.r14
    }
    fn set_r14(&mut self, r14: usize) {
        self.r14 = r14;
    }
    fn r15(&self) -> usize {
        self.r15
    }
    fn set_r15(&mut self, r15: usize) {
        self.r15 = r15;
    }
    fn rbp(&self) -> usize {
        self.rbp
    }
    fn set_rbp(&mut self, rbp: usize) {
        self.rbp = rbp;
    }
}
