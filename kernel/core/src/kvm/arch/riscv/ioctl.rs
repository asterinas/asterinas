use crate::{
    prelude::*,
    util::ioctl::{InData, InOutData, RawIoctl},
    ioc,
};

const KVM_CAP_MP_STATE: usize = 14;
const KVM_CAP_IMMEDIATE_EXIT: usize = 136;

pub(super) type Interrupt = ioc!(KVM_INTERRUPT, 0xAE, 0x86, InData<KvmInterrupt>);
pub(super) type GetRegList = ioc!(KVM_GET_REG_LIST, 0xAE, 0xB0, InOutData<RegList>);
pub(super) type SetMpState = ioc!(KVM_SET_MP_STATE, 0xAE, 0x99, InData<MpState>);

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
        KVM_CAP_MP_STATE
        | KVM_CAP_IMMEDIATE_EXIT => 1,

        _ => 0,
    }
}

/// The common `struct kvm_mp_state`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct MpState {
    mp_state: u32,
}
