// SPDX-License-Identifier: BSD-3-Clause
// Copyright(c) 2023-2024 Intel Corporation.

//! The TDVMCALL helps invoke services from the host VMM. From the perspective of the host VMM, the TDVMCALL is a trap-like, VM exit into
//! the host VMM, reported via the SEAMRET instruction flow.
//! By design, after the SEAMRET, the host VMM services the request specified in the parameters
//! passed by the TD during the TDG.VP.VMCALL (that are passed via SEAMRET to the VMM), then
//! resumes the TD via a SEAMCALL [TDH.VP.ENTER] invocation.
extern crate alloc;

use crate::asm::asm_td_vmcall;
use alloc::fmt;
use bitflags::bitflags;
use core::fmt::Write;
use x86_64::{
    registers::rflags::{self, RFlags},
    structures::port::PortRead,
};

/// TDVMCALL Instruction Leaf Numbers Definition.
#[repr(u64)]
pub enum TdVmcallNum {
    Cpuid = 0x0000a,
    Hlt = 0x0000c,
    Io = 0x0001e,
    Rdmsr = 0x0001f,
    Wrmsr = 0x00020,
    RequestMmio = 0x00030,
    Wbinvd = 0x00036,
    Pconfig = 0x00041,
    Mapgpa = 0x10001,
}

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
pub struct CpuIdInfo {
    pub eax: usize,
    pub ebx: usize,
    pub ecx: usize,
    pub edx: usize,
}

pub enum Direction {
    In,
    Out,
}

pub enum Operand {
    Dx,
    Immediate,
}

pub enum IoSize {
    Size1 = 1,
    Size2 = 2,
    Size4 = 4,
    Size8 = 8,
}

pub fn cpuid(eax: u32, ecx: u32) -> Result<CpuIdInfo, TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::Cpuid as u64,
        r12: eax as u64,
        r13: ecx as u64,
        ..Default::default()
    };
    td_vmcall(&mut args)?;
    Ok(CpuIdInfo {
        eax: args.r12 as usize,
        ebx: args.r13 as usize,
        ecx: args.r14 as usize,
        edx: args.r15 as usize,
    })
}

pub fn hlt() {
    let interrupt_blocked = !rflags::read().contains(RFlags::INTERRUPT_FLAG);
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::Hlt as u64,
        r12: interrupt_blocked as u64,
        ..Default::default()
    };
    let _ = td_vmcall(&mut args);
}

/// # Safety
/// Make sure the index is valid.
pub unsafe fn rdmsr(index: u32) -> Result<u64, TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::Rdmsr as u64,
        r12: index as u64,
        ..Default::default()
    };
    td_vmcall(&mut args)?;
    Ok(args.r11)
}

/// # Safety
/// Make sure the index and the corresponding value are valid.
pub unsafe fn wrmsr(index: u32, value: u64) -> Result<(), TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::Wrmsr as u64,
        r12: index as u64,
        r13: value,
        ..Default::default()
    };
    td_vmcall(&mut args)
}

/// Used to help perform WBINVD or WBNOINVD operation.
/// - cache_operation: 0: WBINVD, 1: WBNOINVD
pub fn perform_cache_operation(cache_operation: u64) -> Result<(), TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::Wbinvd as u64,
        r12: cache_operation,
        ..Default::default()
    };
    td_vmcall(&mut args)
}

/// # Safety
/// Make sure the mmio address is valid.
pub unsafe fn read_mmio(size: IoSize, mmio_gpa: &[u8]) -> Result<u64, TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::RequestMmio as u64,
        r12: size as u64,
        r13: 0,
        r14: mmio_gpa.as_ptr() as u64,
        ..Default::default()
    };
    td_vmcall(&mut args)?;
    Ok(args.r11)
}

/// # Safety
/// Make sure the mmio address is valid.
pub unsafe fn write_mmio(size: IoSize, mmio_gpa: &[u8], data: u64) -> Result<(), TdVmcallError> {
    let mut args = TdVmcallArgs {
        r11: TdVmcallNum::RequestMmio as u64,
        r12: size as u64,
        r13: 1,
        r14: mmio_gpa.as_ptr() as u64,
        r15: data,
        ..Default::default()
    };
    td_vmcall(&mut args)
}

macro_rules! io_read {
    ($port:expr, $ty:ty) => {{
        let mut args = TdVmcallArgs {
            r11: TdVmcallNum::Io as u64,
            r12: core::mem::size_of::<$ty>() as u64,
            r13: IO_READ,
            r14: $port as u64,
            ..Default::default()
        };
        td_vmcall(&mut args)?;
        Ok(args.r11 as u32)
    }};
}

pub fn io_read(size: IoSize, port: u16) -> Result<u32, TdVmcallError> {
    match size {
        IoSize::Size1 => io_read!(port, u8),
        IoSize::Size2 => io_read!(port, u16),
        IoSize::Size4 => io_read!(port, u32),
        _ => unreachable!(),
    }
}

macro_rules! io_write {
    ($port:expr, $byte:expr, $size:expr) => {{
        let mut args = TdVmcallArgs {
            r11: TdVmcallNum::Io as u64,
            r12: core::mem::size_of_val(&$byte) as u64,
            r13: IO_WRITE,
            r14: $port as u64,
            r15: $byte as u64,
            ..Default::default()
        };
        td_vmcall(&mut args)
    }};
}

pub fn io_write(size: IoSize, port: u16, byte: u32) -> Result<(), TdVmcallError> {
    match size {
        IoSize::Size1 => io_write!(port, byte, u8),
        IoSize::Size2 => io_write!(port, byte, u16),
        IoSize::Size4 => io_write!(port, byte, u32),
        _ => unreachable!(),
    }
}

fn td_vmcall(args: &mut TdVmcallArgs) -> Result<(), TdVmcallError> {
    let result = unsafe { asm_td_vmcall(args) };
    match result {
        0 => Ok(()),
        _ => Err(result.into()),
    }
}

bitflags! {
    /// LineSts: Line Status
    struct LineSts: u8 {
        const INPUT_FULL = 1;
        const OUTPUT_EMPTY = 1 << 5;
    }
}

fn read_line_sts() -> LineSts {
    LineSts::from_bits_truncate(unsafe { PortRead::read_from_port(SERIAL_LINE_STS) })
}

struct Serial;

impl Serial {
    fn serial_write_byte(byte: u8) {
        match byte {
            // Backspace/Delete
            8 | 0x7F => {
                while !read_line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                io_write!(SERIAL_IO_PORT, 8, u8).unwrap();
                while !read_line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                io_write!(SERIAL_IO_PORT, b' ', u8).unwrap();
                while !read_line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                io_write!(SERIAL_IO_PORT, 8, u8).unwrap();
            }
            _ => {
                while !read_line_sts().contains(LineSts::OUTPUT_EMPTY) {}
                io_write!(SERIAL_IO_PORT, byte, u8).unwrap();
            }
        }
    }
}

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            Serial::serial_write_byte(c);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Serial
        .write_fmt(args)
        .expect("Failed to write to serial port");
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
