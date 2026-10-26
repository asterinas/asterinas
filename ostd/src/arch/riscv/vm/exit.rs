// SPDX-License-Identifier: MPL-2.0
use int_to_c_enum::TryFromInt;

use crate::{
    arch::vm::types::VcpuCsrs,
    arch::vm::csr::*,
};

/// A VM exit that needs handling by the kernel client.
#[derive(Clone, Copy, Debug)]
pub struct GuestExitInfo {
    pub sepc: usize,
    pub scause: usize,
    pub stval: usize,
    pub htval: usize,
    pub htinst: usize,
}

pub const INTERRUPT_BIT: usize = 1usize << (usize::BITS - 1);
pub const SUPERVISOR_SOFTWARE_INTERRUPT: usize = 1;
pub const SUPERVISOR_TIMER_INTERRUPT: usize = 5;
pub const SUPERVISOR_EXTERNAL_INTERRUPT: usize = 9;

/// Synchronous exception codes relevant to guest execution.
#[expect(non_camel_case_types)]
#[repr(usize)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum GuestException {
    INSTRUCTION_ADDRESS_MISALIGNED = 0,
    INSTRUCTION_ACCESS_FAULT = 1,
    ILLEGAL_INSTRUCTION = 2,
    BREAKPOINT = 3,
    LOAD_ADDRESS_MISALIGNED = 4,
    LOAD_ACCESS_FAULT = 5,
    STORE_AMO_ADDRESS_MISALIGNED = 6,
    STORE_AMO_ACCESS_FAULT = 7,

    ENVIRONMENT_CALL_FROM_USER = 8,
    ENVIRONMENT_CALL_FROM_SUPERVISOR = 9,
    ENVIRONMENT_CALL_FROM_VIRTUAL_SUPERVISOR = 10,
    ENVIRONMENT_CALL_FROM_MACHINE = 11,

    INSTRUCTION_PAGE_FAULT = 12,
    LOAD_PAGE_FAULT = 13,
    STORE_AMO_PAGE_FAULT = 15,

    INSTRUCTION_GUEST_PAGE_FAULT = 20,
    LOAD_GUEST_PAGE_FAULT = 21,
    VIRTUAL_INSTRUCTION = 22,
    STORE_AMO_GUEST_PAGE_FAULT = 23,
}

pub(super) fn exit_info(guest_csrs: &VcpuCsrs) -> GuestExitInfo {
    let htval = unsafe { Htval::read() };
    let htinst = unsafe { Htinst::read() };
    
    GuestExitInfo {
        sepc: guest_csrs.sepc,
        scause: riscv::register::scause::read().bits(),
        stval: riscv::register::stval::read(),
        htval,
        htinst,
    }
}
