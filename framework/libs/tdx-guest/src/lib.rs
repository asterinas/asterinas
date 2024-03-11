// SPDX-License-Identifier: BSD-3-Clause
// Copyright(c) 2023-2024 Intel Corporation.

#![cfg_attr(not(test), no_std)]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate alloc;

mod asm;
pub mod tdcall;
pub mod tdvmcall;

use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

use raw_cpuid::{native_cpuid::cpuid_count, CpuIdResult};
use tdcall::{InitError, TdgVpInfo};

pub use self::{
    tdcall::{get_veinfo, TdxVirtualExceptionType},
    tdvmcall::print,
};

static TDX_ENABLED: AtomicBool = AtomicBool::new(false);

#[inline(always)]
pub fn tdx_is_enabled() -> bool {
    TDX_ENABLED.load(Relaxed)
}

pub fn init_tdx() -> Result<TdgVpInfo, InitError> {
    check_tdx_guest()?;
    TDX_ENABLED.store(true, Relaxed);
    Ok(tdcall::get_tdinfo()?)
}

fn check_tdx_guest() -> Result<(), InitError> {
    const TDX_CPUID_LEAF_ID: u64 = 0x21;
    let cpuid_leaf = cpuid_count(0, 0).eax as u64;
    if cpuid_leaf < TDX_CPUID_LEAF_ID {
        return Err(InitError::TdxCpuLeafIdError);
    }
    let cpuid_result: CpuIdResult = cpuid_count(TDX_CPUID_LEAF_ID as u32, 0);
    if &cpuid_result.ebx.to_ne_bytes() != b"Inte"
        || &cpuid_result.edx.to_ne_bytes() != b"lTDX"
        || &cpuid_result.ecx.to_ne_bytes() != b"    "
    {
        return Err(InitError::TdxVendorIdError);
    }
    Ok(())
}
