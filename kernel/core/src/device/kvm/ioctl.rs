use crate::{
    prelude::*,
    util::ioctl::{InData, NoData},
    ioc,
    kvm::ioctl::{UserMemoryRegion, OneReg},
};

pub(super) const KVM_API_VERSION: i32 = 12;

const KVM_COALESCED_MMIO_PAGE_OFFSET: usize = 2;
pub(super)const KVM_RUN_MMAP_SIZE: usize = (KVM_COALESCED_MMIO_PAGE_OFFSET + 1) * PAGE_SIZE;

// KVM _IO commands may still pass scalar values in the ioctl argument.
// The command word itself encodes no direction or data size for them.

// System ioctls(for device.rs)
pub(super) type GetApiVersion = ioc!(KVM_GET_API_VERSION, 0xAE, 0x00, NoData);
pub(super) type CreateVm = ioc!(KVM_CREATE_VM, 0xAE, 0x01, NoData);
pub(super) type CheckExtension = ioc!(KVM_CHECK_EXTENSION, 0xAE, 0x03, NoData);
pub(super) type GetVcpuMmapSize = ioc!(KVM_GET_VCPU_MMAP_SIZE, 0xAE, 0x04, NoData);

// VM ioctls.
pub(super) type CreateVcpu = ioc!(KVM_CREATE_VCPU, 0xAE, 0x41, NoData);
pub(super) type SetUserMemoryRegion = ioc!(
    KVM_SET_USER_MEMORY_REGION,
    0xAE,
    0x46,
    InData<UserMemoryRegion>
);

// VCPU ioctls.
pub(super) type Run = ioc!(KVM_RUN, 0xAE, 0x80, NoData);
pub(super) type GetOneReg = ioc!(KVM_GET_ONE_REG, 0xAE, 0xAB, InData<OneReg>);
pub(super) type SetOneReg = ioc!(KVM_SET_ONE_REG, 0xAE, 0xAC, InData<OneReg>);
pub(super) type GetStatsFd = ioc!(KVM_GET_STATS_FD, 0xAE, 0xCE, NoData);
