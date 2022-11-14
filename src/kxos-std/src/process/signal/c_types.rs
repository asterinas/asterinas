#![allow(non_camel_case_types)]
use core::mem;

use kxos_frame::cpu::GpRegs;

use crate::prelude::*;

use super::sig_num::SigNum;

pub type sigset_t = u64;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct sigaction_t {
    pub handler_ptr: Vaddr,
    pub flags: u32,
    pub restorer_ptr: Vaddr,
    pub mask: sigset_t,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct siginfo_t {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    _padding: i32,
    /// siginfo_fields should be a union type ( See occlum definition ). But union type have unsafe interfaces.
    /// Here we use a simple byte array.
    pub siginfo_fields: [u8; 128 - mem::size_of::<i32>() * 4],
}

impl siginfo_t {
    pub fn new(num: SigNum, code: i32) -> Self {
        let zero_fields = [0u8; 128 - mem::size_of::<i32>() * 4];
        siginfo_t {
            si_signo: num.as_u8() as i32,
            si_errno: 0,
            si_code: code,
            _padding: 0,
            siginfo_fields: zero_fields,
        }
    }
}

#[derive(Clone, Copy, Debug, Pod)]
#[repr(C)]
pub struct ucontext_t {
    pub uc_flags: u64,
    pub uc_link: Vaddr, // *mut ucontext_t
    pub uc_stack: stack_t,
    pub uc_mcontext: mcontext_t,
    pub uc_sigmask: sigset_t,
    pub fpregs: [u8; 64 * 8], //fxsave structure
}

impl Default for ucontext_t {
    fn default() -> Self {
        Self {
            uc_flags: Default::default(),
            uc_link: Default::default(),
            uc_stack: Default::default(),
            uc_mcontext: Default::default(),
            uc_sigmask: Default::default(),
            fpregs: [0u8; 64 * 8],
        }
    }
}

pub type stack_t = sigaltstack_t;

#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct sigaltstack_t {
    pub ss_sp: Vaddr, // *mut c_void
    pub ss_flags: i32,
    pub ss_size: usize,
}

#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct mcontext_t {
    pub inner: SignalCpuContext,
    // TODO: the fields should be csgsfs, err, trapno, oldmask, and cr2
    _unused0: [u64; 5],
    // TODO: this field should be `fpregs: fpregset_t,`
    _unused1: usize,
    _reserved: [u64; 8],
}

#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct SignalCpuContext {
    pub gp_regs: GpRegs,
    pub fpregs_on_heap: u64,
    pub fpregs: Vaddr, // *mut FpRegs,
}
