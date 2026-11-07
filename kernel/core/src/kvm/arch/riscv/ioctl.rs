// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    util::ioctl::{InData, InOutData, RawIoctl},
    ioc,
};

// Extension capability list.
const KVM_CAP_IMMEDIATE_EXIT: usize = 136;

pub(super) type Interrupt = ioc!(KVM_INTERRUPT, 0xAE, 0x86, InData<KvmInterrupt>);
pub(super) type GetRegList = ioc!(KVM_GET_REG_LIST, 0xAE, 0xB0, InOutData<RegList>);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct KvmInterrupt {
    pub(super) irq: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RegList {
    /// The number of regs
    pub(super) n: usize,
    // Here is a FLEX_ARRAY, but in rust we do not write it
}

pub(in crate::kvm) fn check_arch_extension(raw_ioctl: RawIoctl) -> i32 {
    match raw_ioctl.arg() {
        KVM_CAP_IMMEDIATE_EXIT => 1,

        _ => 0,
    }
}
