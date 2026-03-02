// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(non_camel_case_types)]

use inherit_methods_macro::inherit_methods;
use ostd::arch::cpu::context::UserContext;

use super::sig_num::SigNum;
use crate::{
    arch::cpu::SigContext,
    prelude::*,
    process::{Pid, Uid},
};

pub type sigset_t = u64;
// FIXME: this type should be put at suitable place
pub type clock_t = i64;

#[repr(C)]
#[padding_struct]
#[derive(Debug, Clone, Copy, Default, Pod)]
pub struct sigaction_t {
    pub handler_ptr: Vaddr,
    pub flags: u32,
    pub restorer_ptr: Vaddr,
    pub mask: sigset_t,
}

#[repr(C)]
#[padding_struct]
#[derive(Clone, Copy, Pod, Default)]
pub struct siginfo_t {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    // In x86_64, there will be a 4-bytes padding here automatically, the offset of `siginfo_fields` is `0x10`.
    // Yet in other architectures like arm64, there is no padding here and the offset of `siginfo_fields` is `0x0c`.
    //_padding: i32,
    siginfo_fields: siginfo_fields_t,
}

impl siginfo_t {
    pub fn new(num: SigNum, code: i32) -> Self {
        siginfo_t {
            si_signo: num.as_u8() as i32,
            si_errno: 0,
            si_code: code,
            siginfo_fields: siginfo_fields_t::zero_fields(),
            ..Default::default()
        }
    }

    pub fn set_si_addr(&mut self, si_addr: Vaddr) {
        self.siginfo_fields.sigfault_mut().addr = si_addr;
    }

    pub fn set_pid_uid(&mut self, pid: Pid, uid: Uid) {
        let pid_uid = {
            let pid_uid = siginfo_piduid_t { pid, uid };
            siginfo_common_first_t::new_piduid(pid_uid)
        };

        self.siginfo_fields.common_mut().first = pid_uid;
    }

    pub fn set_status(&mut self, status: i32) {
        *self
            .siginfo_fields
            .common_mut()
            .second
            .sigchild_mut()
            .status_mut() = status;
    }

    pub fn si_addr(&self) -> Vaddr {
        self.siginfo_fields.sigfault().addr
    }
}

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
union siginfo_fields_t {
    bytes: [u8; 128 - size_of::<i32>() * 4],
    common: siginfo_common_t,
    sigfault: siginfo_sigfault_t,
}

impl Default for siginfo_fields_t {
    fn default() -> Self {
        Self::new_zeroed()
    }
}

impl siginfo_fields_t {
    fn zero_fields() -> Self {
        Self::new_zeroed()
    }
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
struct siginfo_common_t {
    first: siginfo_common_first_t,
    second: siginfo_common_second_t,
}

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
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

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
union siginfo_common_second_t {
    value: sigval_t,
    sigchild: siginfo_sigchild_t,
}

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
pub union sigval_t {
    sigval_int: i32,
    sigval_ptr: Vaddr, //*mut c_void
}

impl sigval_t {
    pub fn read_int(&self) -> i32 {
        *self.sigval_int()
    }

    pub fn read_ptr(&self) -> Vaddr {
        *self.sigval_ptr()
    }
}

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
union siginfo_sigchild_t {
    status: i32,
    utime: clock_t,
    stime: clock_t,
}

#[repr(C)]
#[padding_struct]
#[derive(Clone, Copy, Pod)]
struct siginfo_sigfault_t {
    addr: Vaddr, //*const c_void
    addr_lsb: i16,
    first: siginfo_sigfault_first_t,
}

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
union siginfo_sigfault_first_t {
    addr_bnd: siginfo_addr_bnd_t,
    pkey: u32,
}

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
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
#[padding_struct]
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

// FIXME: Currently Rust generates array impls for every size up to 32 manually
// and there is ongoing work on refactoring with const generics. We can just
// derive the `Default` implementation once that is done.
//
// See <https://github.com/rust-lang/rust/issues/61415>.
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
impl Default for ucontext_t {
    fn default() -> Self {
        Self {
            uc_flags: 0,
            uc_link: Default::default(),
            uc_stack: Default::default(),
            uc_sigmask: Default::default(),
            __unused: [0; 120],
            uc_mcontext: Default::default(),
            __pad1: [0; _],
            __pad2: [0; _],
            __pad3: [0; _],
            __pad4: [0; _],
            __pad5: [0; _],
            __pad6: [0; _],
        }
    }
}

pub type stack_t = sigaltstack_t;

#[repr(C)]
#[padding_struct]
#[derive(Debug, Clone, Copy, Pod, Default)]
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

#[repr(C)]
#[pod_union]
#[derive(Clone, Copy)]
pub union _sigev_un {
    pub _pad: [i32; SIGEV_PAD_SIZE],
    pub _tid: i32,
    pub _sigev_thread: _sigev_thread,
}

impl _sigev_un {
    pub fn read_tid(&self) -> i32 {
        *self._tid()
    }

    pub fn read_function(&self) -> Vaddr {
        self._sigev_thread().function
    }

    pub fn read_attribute(&self) -> Vaddr {
        self._sigev_thread().attribute
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

#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub struct sigevent_t {
    pub sigev_value: sigval_t,
    pub sigev_signo: i32,
    pub sigev_notify: i32,
    pub sigev_un: _sigev_un,
}
