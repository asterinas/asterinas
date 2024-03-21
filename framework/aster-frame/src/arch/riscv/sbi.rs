// SPDX-License-Identifier: MPL-2.0

//! Provide SBI (Supervisor Binary Interface) calls for RISC-V.

#![allow(dead_code)]

use core::arch::asm;

use num_derive::{FromPrimitive, ToPrimitive};

#[derive(FromPrimitive, ToPrimitive, Clone, Copy, Debug)]
#[repr(isize)]
pub enum SBIRetCode {
    Success = 0,
    Failed = -1,
    NotSupported = -2,
    InvalidParam = -3,
    Denied = -4,
    InvalidAddress = -5,
    AlreadyAvailable = -6,
}

#[derive(Clone, Copy, Debug)]
pub struct SBIRet {
    error: SBIRetCode,
    value: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct SBICall {
    eid: usize,
    fid: usize,
}

fn sbi_call(call: SBICall, arg0: usize, arg1: usize, arg2: usize) -> SBIRet {
    let error;
    let value;
    unsafe {
        asm!(
            "ecall",
            in("a0") arg0, in("a1") arg1,
            in("a6") call.fid, in("a7") call.eid,
            lateout("a0") error, lateout("a1") value,
        )
    }
    SBIRet {
        error: num::FromPrimitive::from_isize(error).unwrap(),
        value: value,
    }
}

pub mod srst {
    use num_derive::{FromPrimitive, ToPrimitive};

    use super::{sbi_call, SBICall, SBIRet};

    const SBI_EID_SRST: usize = 0x53525354;
    const SBI_FID_SRST_SYSTEM_RESET: usize = 0;

    #[derive(FromPrimitive, ToPrimitive, Clone, Copy, Debug)]
    #[repr(u32)]
    pub enum ResetType {
        Shutdown = 0,
        ColdReboot = 1,
        WarmReboot = 2,
    }

    #[derive(FromPrimitive, ToPrimitive, Clone, Copy, Debug)]
    #[repr(u32)]
    pub enum ResetReason {
        NoReason = 0,
        SystemFailure = 1,
    }

    pub fn system_reset(reset_type: ResetType, reset_reason: ResetReason) -> SBIRet {
        sbi_call(
            SBICall {
                eid: SBI_EID_SRST,
                fid: SBI_FID_SRST_SYSTEM_RESET,
            },
            reset_type.to_u32() as usize,
            reset_reason.to_u32() as usize,
            0,
        )
    }
}

pub mod hsm {
    use super::{sbi_call, SBICall, SBIRet};

    const SBI_EID_HSM: usize = 0x48534D;
    const SBI_FID_HSM_START: usize = 0;
    const SBI_FID_HSM_STOP: usize = 1;
    const SBI_FID_HSM_STATUS: usize = 2;

    pub fn sbi_hart_start(hartid: usize, start_addr: usize, opaque: usize) -> SBIRet {
        sbi_call(
            SBICall {
                eid: SBI_EID_HSM,
                fid: SBI_FID_HSM_START,
            },
            hartid,
            start_addr as usize,
            opaque,
        )
    }
    pub fn sbi_hart_stop() -> ! {
        sbi_call(
            SBICall {
                eid: SBI_EID_HSM,
                fid: SBI_FID_HSM_STOP,
            },
            0,
            0,
            0,
        );
        unreachable!();
    }
    pub fn sbi_hart_get_status(hartid: usize) -> SBIRet {
        sbi_call(
            SBICall {
                eid: SBI_EID_HSM,
                fid: SBI_FID_HSM_START,
            },
            hartid,
            0,
            0,
        )
    }
}

pub mod time {
    use super::{sbi_call, SBICall, SBIRet};

    const SBI_EID_TIME: usize = 0x54494D45;
    const SBI_FID_TIME_SET: usize = 0;

    pub fn sbi_set_timer(stime_value: u64) -> SBIRet {
        #[cfg(target_pointer_width = "32")]
        let ret = sbi_call(
            SBICall {
                eid: SBI_EID_TIME,
                fid: SBI_FID_TIME_SET,
            },
            stime_value as usize,
            (stime_value >> 32) as usize,
            0,
        );
        #[cfg(target_pointer_width = "64")]
        let ret = sbi_call(
            SBICall {
                eid: SBI_EID_TIME,
                fid: SBI_FID_TIME_SET,
            },
            stime_value as usize,
            0,
            0,
        );
        ret
    }
}

/// Legacy SBI calls.
pub mod legacy {
    use core::arch::asm;

    const SBI_SET_TIMER: usize = 0;
    const SBI_CONSOLE_PUTCHAR: usize = 1;
    const SBI_CONSOLE_GETCHAR: usize = 2;
    const SBI_CLEAR_IPI: usize = 3;
    const SBI_SEND_IPI: usize = 4;
    const SBI_REMOTE_FENCE_I: usize = 5;
    const SBI_REMOTE_SFENCE_VMA: usize = 6;
    const SBI_REMOTE_SFENCE_VMA_ASID: usize = 7;
    const SBI_SHUTDOWN: usize = 8;

    fn sbi_call_legacy(call: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
        let ret;
        unsafe {
            asm!(
                "ecall",
                in("a0") arg0, in("a1") arg1, in("a2") arg2,
                in("a7") call,
                lateout("a0") ret,
            );
        }
        ret
    }

    pub fn console_putchar(ch: usize) {
        sbi_call_legacy(SBI_CONSOLE_PUTCHAR, ch, 0, 0);
    }

    pub fn console_getchar() -> usize {
        sbi_call_legacy(SBI_CONSOLE_GETCHAR, 0, 0, 0)
    }

    pub fn shutdown() -> ! {
        sbi_call_legacy(SBI_SHUTDOWN, 0, 0, 0);
        unreachable!()
    }

    pub fn set_timer(stime_value: u64) {
        #[cfg(target_pointer_width = "32")]
        sbi_call_legacy(
            SBI_SET_TIMER,
            stime_value as usize,
            (stime_value >> 32) as usize,
            0,
        );
        #[cfg(target_pointer_width = "64")]
        sbi_call_legacy(SBI_SET_TIMER, stime_value as usize, 0, 0);
    }

    pub fn clear_ipi() {
        sbi_call_legacy(SBI_CLEAR_IPI, 0, 0, 0);
    }

    pub fn send_ipi(hart_mask: usize) {
        sbi_call_legacy(SBI_SEND_IPI, &hart_mask as *const _ as usize, 0, 0);
    }

    pub fn remote_fence_i(hart_mask: usize) {
        sbi_call_legacy(SBI_REMOTE_FENCE_I, &hart_mask as *const _ as usize, 0, 0);
    }

    pub fn remote_sfence_vma(hart_mask: usize, _start: usize, _size: usize) {
        sbi_call_legacy(SBI_REMOTE_SFENCE_VMA, &hart_mask as *const _ as usize, 0, 0);
    }

    pub fn remote_sfence_vma_asid(hart_mask: usize, _start: usize, _size: usize, _asid: usize) {
        sbi_call_legacy(
            SBI_REMOTE_SFENCE_VMA_ASID,
            &hart_mask as *const _ as usize,
            0,
            0,
        );
    }
}
