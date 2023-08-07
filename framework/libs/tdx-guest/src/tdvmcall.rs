extern crate alloc;

use crate::{asm::asm_td_vmcall, tdcall::TdgVeInfo, TdxTrapFrame};
use alloc::fmt;
use bitflags::bitflags;
use core::fmt::Write;
use x86_64::{
    registers::rflags::{self, RFlags},
    structures::port::PortRead,
};

const TDVMCALL_CPUID: u64 = 0x0000a;
const TDVMCALL_HLT: u64 = 0x0000c;
const TDVMCALL_IO: u64 = 0x0001e;
const TDVMCALL_RDMSR: u64 = 0x0001f;
const TDVMCALL_WRMSR: u64 = 0x00020;
const TDVMCALL_REQUEST_MMIO: u64 = 0x00030;
const TDVMCALL_WBINVD: u64 = 0x00036;
const TDVMCALL_PCONFIG: u64 = 0x00041;
const TDVMCALL_MAPGPA: u64 = 0x10001;

const SERIAL_IO_PORT: u16 = 0x3F8;
const SERIAL_LINE_STS: u16 = 0x3FD;
const IO_READ: u64 = 0;
const IO_WRITE: u64 = 1;

#[derive(Debug, PartialEq)]
pub enum TdVmcallError {
    /// TDCALL[TDG.VP.VMCALL] sub-function invocation must be retried.
    TdxRetry,
    /// Invalid operand to TDG.VP.VMCALL sub-function.
    TdxInvalidOperand,
    /// GPA already mapped.
    TdxGpaInuse,
    /// Operand (address) aligned error.
    TdxAlignError,
    Other,
}

impl From<u64> for TdVmcallError {
    fn from(val: u64) -> Self {
        match val {
            0x1 => Self::TdxRetry,
            0x8000_0000_0000_0000 => Self::TdxInvalidOperand,
            0x8000_0000_0000_0001 => Self::TdxGpaInuse,
            0x8000_0000_0000_0002 => Self::TdxAlignError,
            _ => Self::Other,
        }
    }
}

#[repr(C)]
#[derive(Default)]
pub(crate) struct TdVmcallArgs {
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

#[repr(C)]
#[derive(Debug, Default)]
pub(crate) struct CpuIdInfo {
    pub eax: usize,
    pub ebx: usize,
    pub ecx: usize,
    pub edx: usize,
}

enum Direction {
    In,
    Out,
}

enum Operand {
    Dx,
    Immediate,
}

pub(crate) fn tdvmcall_cpuid(eax: u32, ecx: u32) -> Result<CpuIdInfo, TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_CPUID,
        r12: eax as u64,
        r13: ecx as u64,
        ..Default::default()
    };
    match td_vmcall(&mut args) {
        Ok(()) => Ok(CpuIdInfo {
            eax: args.r12 as usize,
            ebx: args.r13 as usize,
            ecx: args.r14 as usize,
            edx: args.r15 as usize,
        }),
        Err(res) => Err(res),
    }
}

pub(crate) fn tdvmcall_hlt() {
    let interrupt_blocked = !rflags::read().contains(RFlags::INTERRUPT_FLAG);
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_HLT,
        r12: interrupt_blocked as u64,
        ..Default::default()
    };
    let _ = td_vmcall(&mut args);
}

pub(crate) fn tdvmcall_rdmsr(index: u32) -> Result<u64, TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_RDMSR,
        r12: index as u64,
        ..Default::default()
    };
    match td_vmcall(&mut args) {
        Ok(()) => Ok(args.r11),
        Err(res) => Err(res),
    }
}

pub(crate) fn tdvmcall_wrmsr(index: u32, value: u64) -> Result<(), TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_WRMSR,
        r12: index as u64,
        r13: value,
        ..Default::default()
    };
    match td_vmcall(&mut args) {
        Ok(()) => Ok(()),
        Err(res) => Err(res),
    }
}

/// Used to help perform WBINVD operation.
pub(crate) fn tdvmcall_wbinvd(wbinvd: u64) -> Result<(), TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_WBINVD,
        r12: wbinvd,
        ..Default::default()
    };
    match td_vmcall(&mut args) {
        Ok(()) => Ok(()),
        Err(res) => Err(res),
    }
}

pub(crate) fn tdvmcall_read_mmio(size: u64, mmio_addr: u64) -> Result<u64, TdVmcallError> {
    match size {
        1 | 2 | 4 | 8 => {}
        _ => return Err(TdVmcallError::TdxInvalidOperand),
    }
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_REQUEST_MMIO,
        r12: size,
        r13: 0,
        r14: mmio_addr,
        ..Default::default()
    };
    match td_vmcall(&mut args) {
        Ok(()) => Ok(args.r11),
        Err(res) => Err(res),
    }
}

pub(crate) fn tdvmcall_write_mmio(
    size: u64,
    mmio_addr: u64,
    data: u64,
) -> Result<(), TdVmcallError> {
    match size {
        1 | 2 | 4 | 8 => {}
        _ => {
            return Err(TdVmcallError::TdxInvalidOperand);
        }
    }
    let mut args = TdVmcallArgs {
        r11: TDVMCALL_REQUEST_MMIO,
        r12: size,
        r13: 1,
        r14: mmio_addr,
        r15: data,
        ..Default::default()
    };
    match td_vmcall(&mut args) {
        Ok(()) => Ok(()),
        Err(res) => Err(res),
    }
}

pub(crate) fn tdvmcall_io(trapframe: &mut impl TdxTrapFrame, ve_info: &TdgVeInfo) -> bool {
    let size = match ve_info.exit_qualification & 0x3 {
        0 => 1,
        1 => 2,
        3 => 4,
        _ => panic!("Invalid size value"),
    };
    let direction = if (ve_info.exit_qualification >> 3) & 0x1 == 0 {
        Direction::Out
    } else {
        Direction::In
    };
    let string = (ve_info.exit_qualification >> 4) & 0x1 == 1;
    let repeat = (ve_info.exit_qualification >> 5) & 0x1 == 1;
    let operand = if (ve_info.exit_qualification >> 6) & 0x1 == 0 {
        Operand::Dx
    } else {
        Operand::Immediate
    };
    let port = (ve_info.exit_qualification >> 16) as u16;

    match direction {
        Direction::In => {
            trapframe.set_rax(io_read(size, port).unwrap() as usize);
        }
        Direction::Out => {
            io_write(size, port, trapframe.rax() as u32).unwrap();
        }
    };
    true
}

macro_rules! tdvmcall_io_read {
    ($port:expr, $ty:ty) => {{
        let mut args = TdVmcallArgs {
            r11: TDVMCALL_IO,
            r12: core::mem::size_of::<$ty>() as u64,
            r13: IO_READ,
            r14: $port as u64,
            ..Default::default()
        };
        match td_vmcall(&mut args) {
            Ok(()) => Ok(args.r11 as u32),
            Err(res) => Err(res),
        }
    }};
}

fn io_read(size: usize, port: u16) -> Result<u32, TdVmcallError> {
    match size {
        1 => tdvmcall_io_read!(port, u8),
        2 => tdvmcall_io_read!(port, u16),
        4 => tdvmcall_io_read!(port, u32),
        _ => unreachable!(),
    }
}

macro_rules! tdvmcall_io_write {
    ($port:expr, $byte:expr, $size:expr) => {{
        let mut args = TdVmcallArgs {
            r11: TDVMCALL_IO,
            r12: core::mem::size_of_val(&$byte) as u64,
            r13: IO_WRITE,
            r14: $port as u64,
            r15: $byte as u64,
            ..Default::default()
        };
        match td_vmcall(&mut args) {
            Ok(()) => Ok(()),
            Err(res) => Err(res),
        }
    }};
}

fn io_write(size: usize, port: u16, byte: u32) -> Result<(), TdVmcallError> {
    match size {
        1 => tdvmcall_io_write!(port, byte, u8),
        2 => tdvmcall_io_write!(port, byte, u16),
        4 => tdvmcall_io_write!(port, byte, u32),
        _ => unreachable!(),
    }
}

fn td_vmcall(args: &mut TdVmcallArgs) -> Result<(), TdVmcallError> {
    let td_vmcall_result = unsafe { asm_td_vmcall(args) };
    if td_vmcall_result == 0 {
        Ok(())
    } else {
        Err(td_vmcall_result.into())
    }
}

bitflags! {
  struct LineSts: u8 {
    const INPUT_FULL = 1;
    const OUTPUT_EMPTY = 1 << 5;
  }
}

fn line_sts() -> LineSts {
    LineSts::from_bits_truncate(unsafe { PortRead::read_from_port(SERIAL_LINE_STS) })
}

struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            serial_write_byte(c);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Serial
        .write_fmt(args)
        .expect("Failed to write to serial port");
}

fn serial_write_byte(byte: u8) {
    match byte {
        8 | 0x7F => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            tdvmcall_io_write!(SERIAL_IO_PORT, 8, u8).unwrap();
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            tdvmcall_io_write!(SERIAL_IO_PORT, b' ', u8).unwrap();
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            tdvmcall_io_write!(SERIAL_IO_PORT, 8, u8).unwrap();
        }
        _ => {
            while !line_sts().contains(LineSts::OUTPUT_EMPTY) {}
            tdvmcall_io_write!(SERIAL_IO_PORT, byte, u8).unwrap();
        }
    }
}

#[macro_export]
macro_rules! serial_print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::tdvmcall::print(format_args!($fmt $(, $($arg)+)?));
    }
}

#[macro_export]
macro_rules! serial_println {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::tdvmcall::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
  }
}
