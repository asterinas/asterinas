// SPDX-License-Identifier: MPL-2.0

//! Minimal ioctl definitions compatible with the Linux KVM API.

use crate::{
    prelude::*,
    util::ioctl::{InData, NoData, ioc},
};

pub(super) const KVM_API_VERSION: i32 = 12;
pub(super) const KVM_IRQFD_FLAG_DEASSIGN: u32 = 1 << 0;

pub(super) type GetApiVersion = ioc!(KVM_GET_API_VERSION, 0xAE, 0x00, NoData);
pub(super) type CreateVm = ioc!(KVM_CREATE_VM, 0xAE, 0x01, NoData);
pub(super) type IrqFd = ioc!(KVM_IRQFD, 0xAE, 0x76, InData<IrqFdConfig>);

/// The common `struct kvm_irqfd`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct IrqFdConfig {
    pub fd: u32,
    pub gsi: u32,
    pub flags: u32,
    pub resamplefd: u32,
    pub pad: [u8; 16],
}
