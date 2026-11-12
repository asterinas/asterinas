// SPDX-License-Identifier: MPL-2.0

//! Common definitions for the supported KVM userspace ABI.

use crate::{
    ioc,
    prelude::*,
    util::ioctl::{InData, NoData},
};

pub(super) const KVM_API_VERSION: i32 = 12;

const KVM_COALESCED_MMIO_PAGE_OFFSET: usize = 2;
pub(super) const KVM_RUN_MMAP_SIZE: usize = (KVM_COALESCED_MMIO_PAGE_OFFSET + 1) * PAGE_SIZE;

pub(super) const MAX_MEMSLOTS: u32 = 32;

// System ioctls.
pub(super) type GetApiVersion = ioc!(KVM_GET_API_VERSION, 0xae, 0x00, NoData);
pub(super) type CreateVm = ioc!(KVM_CREATE_VM, 0xae, 0x01, NoData);
pub(super) type CheckExtension = ioc!(KVM_CHECK_EXTENSION, 0xae, 0x03, NoData);
pub(super) type GetVcpuMmapSize = ioc!(KVM_GET_VCPU_MMAP_SIZE, 0xae, 0x04, NoData);

// VM ioctls.
pub(super) type CreateVcpu = ioc!(KVM_CREATE_VCPU, 0xae, 0x41, NoData);
pub(super) type SetUserMemoryRegion = ioc!(KVM_SET_USER_MEMORY_REGION, 0xae, 0x46, InData<UserMemoryRegion>);

// VCPU ioctls.
pub(super) type Run = ioc!(KVM_RUN, 0xae, 0x80, NoData);

// Offsets within `struct kvm_run`.
pub(crate) const KVM_RUN_IMMEDIATE_EXIT_OFFSET: usize = 1;
pub(crate) const KVM_RUN_EXIT_REASON_OFFSET: usize = 8;
pub(crate) const KVM_RUN_IO_DIRECTION_OFFSET: usize = 32;
pub(crate) const KVM_RUN_IO_SIZE_OFFSET: usize = 33;
pub(crate) const KVM_RUN_IO_PORT_OFFSET: usize = 34;
pub(crate) const KVM_RUN_IO_COUNT_OFFSET: usize = 36;
pub(crate) const KVM_RUN_IO_DATA_OFFSET_OFFSET: usize = 40;

// Exit reasons for `KVM_RUN`.
pub(crate) const KVM_EXIT_IO: u32 = 2;
pub(crate) const KVM_EXIT_HLT: u32 = 5;
pub(crate) const KVM_EXIT_IO_OUT: u8 = 1;

pub(super) fn check_extension(cap: usize) -> i32 {
    const KVM_CAP_HLT: usize = 1;
    const KVM_CAP_USER_MEMORY: usize = 3;
    const KVM_CAP_SET_TSS_ADDR: usize = 4;
    const KVM_CAP_NR_VCPUS: usize = 9;
    const KVM_CAP_NR_MEMSLOTS: usize = 10;
    const KVM_CAP_DESTROY_MEMORY_REGION_WORKS: usize = 21;
    const KVM_CAP_MAX_VCPUS: usize = 66;
    const KVM_CAP_MAX_VCPU_ID: usize = 128;
    const KVM_CAP_IMMEDIATE_EXIT: usize = 136;

    match cap {
        KVM_CAP_HLT
        | KVM_CAP_USER_MEMORY
        | KVM_CAP_SET_TSS_ADDR
        | KVM_CAP_NR_VCPUS
        | KVM_CAP_DESTROY_MEMORY_REGION_WORKS
        | KVM_CAP_MAX_VCPUS
        | KVM_CAP_MAX_VCPU_ID
        | KVM_CAP_IMMEDIATE_EXIT => 1,
        KVM_CAP_NR_MEMSLOTS => MAX_MEMSLOTS as i32,
        _ => 0,
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct UserMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}
