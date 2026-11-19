// SPDX-License-Identifier: MPL-2.0

//! Ioctl api compatible with Linux KVM.
//! KVM api: https://www.kernel.org/doc/html/latest/virt/kvm/api.html

use crate::{
    prelude::*,
    util::ioctl::RawIoctl,
};

use super::arch;

// Memory options
pub(super) const KVM_MEM_READONLY: u32 = 1 << 1;
const KVM_MAX_NR_MEMSLOTS: i32 = 32;

// Insn options
pub(super) const INSN_OPCODE_SYSTEM: usize = 28;

// Vcpu options
const KVM_MAX_VCPUS: i32 = 64;

// Extension capability list.
const KVM_CAP_USER_MEMORY: usize = 3;
const KVM_CAP_NR_VCPUS: usize = 9;
const KVM_CAP_NR_MEMSLOTS: usize = 10;
const KVM_CAP_COALESCED_MMIO: usize = 15;
const KVM_CAP_DESTROY_MEMORY_REGION_WORKS: usize = 21;
const KVM_CAP_JOIN_MEMORY_REGIONS_WORKS: usize = 30;
const KVM_CAP_IOEVENTFD: usize = 36;
const KVM_CAP_INTERNAL_ERROR_DATA: usize = 40;
const KVM_CAP_MAX_VCPUS: usize = 66;
const KVM_CAP_IOEVENTFD_ANY_LENGTH: usize = 122;
const KVM_CAP_COALESCED_PIO: usize = 162;

// The offset of KVM_RUN(struct KvmRun in run_page.rs)
pub(super) const KVM_RUN_IMMEDIATE_EXIT_OFFSET: usize = 1;
pub(super) const KVM_RUN_EXIT_REASON_OFFSET: usize = 8;
pub(super) const KVM_RUN_MMIO_PHYS_ADDR_OFFSET: usize = 32;
pub(super) const KVM_RUN_MMIO_DATA_OFFSET: usize = 40;
pub(super) const KVM_RUN_MMIO_LEN_OFFSET: usize = 48;
pub(super) const KVM_RUN_MMIO_IS_WRITE_OFFSET: usize = 52;

// The exit reason of KVM_RUN
pub(super) const KVM_EXIT_UNKNOWN: u32 = 0;
pub(super) const KVM_EXIT_MMIO: u32 = 6;
pub(super) const KVM_EXIT_RISCV_SBI: u32 = 35;
pub(super) const KVM_EXIT_RISCV_CSR: u32 = 36;

// Exception causes of vm
pub(super) const EXC_SUPERVISOR_SYSCALL: usize = 10;
pub(super) const EXC_INST_GUEST_PAGE_FAULT: usize = 20;
pub(super) const EXC_LOAD_GUEST_PAGE_FAULT: usize = 21;
pub(super) const EXC_VIRTUAL_INST_FAULT: usize = 22;
pub(super) const EXC_STORE_GUEST_PAGE_FAULT: usize = 23;

pub fn check_extension(raw_ioctl: RawIoctl) -> i32 {
    match raw_ioctl.arg() {
        KVM_CAP_IOEVENTFD
        | KVM_CAP_USER_MEMORY
        | KVM_CAP_DESTROY_MEMORY_REGION_WORKS
        | KVM_CAP_COALESCED_MMIO
        | KVM_CAP_COALESCED_PIO
        | KVM_CAP_JOIN_MEMORY_REGIONS_WORKS
        | KVM_CAP_INTERNAL_ERROR_DATA
        | KVM_CAP_IOEVENTFD_ANY_LENGTH => 1,
        KVM_CAP_NR_VCPUS => 1,
        KVM_CAP_MAX_VCPUS => KVM_MAX_VCPUS,
        KVM_CAP_NR_MEMSLOTS => KVM_MAX_NR_MEMSLOTS,

        _ => arch::ioctl::check_arch_extension(raw_ioctl),
    }
}

/// `struct kvm_userspace_memory_region`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UserMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

/// Struct `OneReg`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct OneReg {
    /// regid
	pub id: usize,
    /// the addr we should read from / write to
	pub addr: usize,
}
