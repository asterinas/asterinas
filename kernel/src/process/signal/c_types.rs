// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(non_camel_case_types)]

use core::mem::{self, size_of};

use aster_util::read_union_field;
use inherit_methods_macro::inherit_methods;
use ostd::cpu::context::UserContext;

use super::sig_num::SigNum;
use crate::{
    arch::cpu::SigContext,
    prelude::*,
    process::{Pid, Uid},
};

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
    // In x86_64, there will be a 4-bytes padding here automatically, the offset of `siginfo_fields` is `0x10`.
    // Yet in other architectures like arm64, there is no padding here and the offset of `siginfo_fields` is `0x0c`.
    //_padding: i32,
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
            siginfo_fields: siginfo_fields_t::zero_fields(),
        }
    }

    pub fn set_si_addr(&mut self, si_addr: Vaddr) {
        self.siginfo_fields.sigfault.addr = si_addr;
    }

    pub fn set_pid_uid(&mut self, pid: Pid, uid: Uid) {
        let pid_uid = siginfo_common_first_t {
            piduid: siginfo_piduid_t { pid, uid },
        };

        self.siginfo_fields.common.first = pid_uid;
    }

    pub fn set_status(&mut self, status: i32) {
        self.siginfo_fields.common.second.sigchild.status = status;
    }

    pub fn si_addr(&self) -> Vaddr {
        read_union_field!(self, Self, siginfo_fields.sigfault.addr)
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
struct siginfo_common_t {
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

impl sigval_t {
    pub fn read_int(&self) -> i32 {
        read_union_field!(self, Self, sigval_int)
    }

    pub fn read_ptr(&self) -> Vaddr {
        read_union_field!(self, Self, sigval_ptr)
    }
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

/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/include/uapi/asm-generic/ucontext.h#L5>
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy, Debug, Default, Pod)]
#[repr(C)]
pub struct ucontext_t {
    pub uc_flags: u64,
    pub uc_link: Vaddr, // *mut ucontext_t
    pub uc_stack: stack_t,
    pub uc_mcontext: mcontext_t,
    pub uc_sigmask: sigset_t,
}

/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/riscv/include/uapi/asm/ucontext.h>
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/include/uapi/asm/ucontext.h>
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
#[derive(Clone, Copy, Debug, Pod)]
#[repr(C)]
pub struct ucontext_t {
    pub uc_flags: u64,
    pub uc_link: Vaddr, // *mut ucontext_t
    pub uc_stack: stack_t,
    pub uc_sigmask: sigset_t,
    pub __unused: [u8; 120],
    pub uc_mcontext: mcontext_t,
}

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
impl Default for ucontext_t {
    fn default() -> Self {
        Self {
            __unused: [0; 120],
            ..Default::default()
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
    inner: SigContext,
}

#[inherit_methods(from = "self.inner")]
impl mcontext_t {
    pub fn copy_user_regs_to(&self, context: &mut UserContext);
    pub fn copy_user_regs_from(&mut self, context: &UserContext);
    #[cfg(target_arch = "x86_64")]
    pub fn fpu_context_addr(&self) -> Vaddr;
    #[cfg(target_arch = "x86_64")]
    pub fn set_fpu_context_addr(&mut self, addr: Vaddr);
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct _sigev_thread {
    pub function: Vaddr,
    pub attribute: Vaddr,
}

const SIGEV_MAX_SIZE: usize = 64;
/// The total size of the fields `sigev_value`, `sigev_signo` and `sigev_notify`.
const SIGEV_PREAMBLE_SIZE: usize = size_of::<i32>() * 2 + size_of::<sigval_t>();
const SIGEV_PAD_SIZE: usize = (SIGEV_MAX_SIZE - SIGEV_PREAMBLE_SIZE) / size_of::<i32>();

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub union _sigev_un {
    pub _pad: [i32; SIGEV_PAD_SIZE],
    pub _tid: i32,
    pub _sigev_thread: _sigev_thread,
}

impl _sigev_un {
    pub fn read_tid(&self) -> i32 {
        read_union_field!(self, Self, _tid)
    }

    pub fn read_function(&self) -> Vaddr {
        read_union_field!(self, Self, _sigev_thread.function)
    }

    pub fn read_attribute(&self) -> Vaddr {
        read_union_field!(self, Self, _sigev_thread.attribute)
    }
}

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(i32)]
pub enum SigNotify {
    SIGEV_SIGNAL = 0,
    SIGEV_NONE = 1,
    SIGEV_THREAD = 2,
    SIGEV_THREAD_ID = 4,
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct sigevent_t {
    pub sigev_value: sigval_t,
    pub sigev_signo: i32,
    pub sigev_notify: i32,
    pub sigev_un: _sigev_un,
}
