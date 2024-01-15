// SPDX-License-Identifier: BSD-3-Clause
// Copyright(c) 2023-2024 Intel Corporation.

use crate::{tdcall::TdcallArgs, tdvmcall::TdVmcallArgs};
use core::arch::global_asm;

global_asm!(include_str!("tdcall.asm"));
global_asm!(include_str!("tdvmcall.asm"));

// TODO: Use sysv64
extern "win64" {
    pub(crate) fn asm_td_call(args: *mut TdcallArgs) -> u64;
    pub(crate) fn asm_td_vmcall(args: *mut TdVmcallArgs) -> u64;
}
