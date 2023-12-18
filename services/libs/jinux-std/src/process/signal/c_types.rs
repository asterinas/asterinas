#![allow(non_camel_case_types)]
use core::mem;

use jinux_frame::cpu::GeneralRegs;
use jinux_util::{read_union_fields, union_read_ptr::UnionReadPtr};

use crate::{
    prelude::*,
    process::{Pid, Uid},
};

use super::sig_num::SigNum;

pub type sigset_t = u64;
// FIXME: this type should be put at suitable place
pub type clock_t = i64;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct sigaction_t {
    pub handler_ptr: Vaddr,
    pub flags: u32,
    pub restorer_ptr: Vaddr,
    pub mask: sigset_t,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct siginfo_t {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    _padding: i32,
    /// siginfo_fields should be a union type ( See occlum definition ). But union type have unsafe interfaces.
    /// Here we use a simple byte array.
    siginfo_fields: siginfo_fields_t,
}

impl siginfo_t {
    pub fn new(num: SigNum, code: i32) -> Self {
        siginfo_t {
            si_signo: num.as_u8() as i32,
            si_errno: 0,
            si_code: code,
            _padding: 0,
            siginfo_fields: siginfo_fields_t::zero_fields(),
        }
    }

    pub fn set_si_addr(&mut self, si_addr: Vaddr) {
        self.siginfo_fields.sigfault.addr = si_addr;
    }

    pub fn si_addr(&self) -> Vaddr {
        // let siginfo = *self;
        read_union_fields!(self.siginfo_fields.sigfault.addr)
    }
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_fields_t {
    bytes: [u8; 128 - mem::size_of::<i32>() * 4],
    common: siginfo_common_t,
    sigfault: siginfo_sigfault_t,
}

impl siginfo_fields_t {
    fn zero_fields() -> Self {
        Self {
            bytes: [0; 128 - mem::size_of::<i32>() * 4],
        }
    }
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_common_t {
    first: siginfo_common_first_t,
    second: siginfo_common_second_t,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_common_first_t {
    piduid: siginfo_piduid_t,
    timer: siginfo_timer_t,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
struct siginfo_piduid_t {
    pid: Pid,
    uid: Uid,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
struct siginfo_timer_t {
    timerid: i32,
    overrun: i32,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_common_second_t {
    value: sigval_t,
    sigchild: siginfo_sigchild_t,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub union sigval_t {
    sigval_int: i32,
    sigval_ptr: Vaddr, //*mut c_void
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_sigchild_t {
    status: i32,
    utime: clock_t,
    stime: clock_t,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
struct siginfo_sigfault_t {
    addr: Vaddr, //*const c_void
    addr_lsb: i16,
    first: siginfo_sigfault_first_t,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_sigfault_first_t {
    addr_bnd: siginfo_addr_bnd_t,
    pkey: u32,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
union siginfo_addr_bnd_t {
    lower: Vaddr, // *const c_void
    upper: Vaddr, // *const c_void,
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
    pub gp_regs: GeneralRegs,
    pub fpregs_on_heap: u64,
    pub fpregs: Vaddr, // *mut FpRegs,
}
