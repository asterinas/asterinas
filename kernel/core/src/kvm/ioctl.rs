// SPDX-License-Identifier: MPL-2.0

//! Ioctl api compatible with Linux KVM.
//! KVM api: https://www.kernel.org/doc/html/latest/virt/kvm/api.html

use crate::{
    prelude::*,
    util::ioctl::RawIoctl,
};

use super::arch::ioctl::check_arch_extension;

pub const KVM_MEM_READONLY: u32 = 1 << 1;

pub const KVM_MAX_VCPUS: i32 = 64;

pub const KVM_CAP_IRQCHIP: usize = 0;
pub const KVM_CAP_USER_MEMORY: usize = 3;
pub const KVM_CAP_NR_VCPUS: usize = 9;
pub const KVM_CAP_NR_MEMSLOTS: usize = 10;
pub const KVM_CAP_COALESCED_MMIO: usize = 15;
pub const KVM_CAP_DESTROY_MEMORY_REGION_WORKS: usize = 21;
pub const KVM_CAP_JOIN_MEMORY_REGIONS_WORKS: usize = 30;
pub const KVM_CAP_IOEVENTFD: usize = 36;
pub const KVM_CAP_INTERNAL_ERROR_DATA: usize = 40;
pub const KVM_CAP_ENABLE_CAP: usize = 54;
pub const KVM_CAP_XSAVE: usize = 55;
pub const KVM_CAP_GET_TSC_KHZ: usize = 61;
pub const KVM_CAP_MAX_VCPUS: usize = 66;
pub const KVM_CAP_ONE_REG: usize = 70;
pub const KVM_CAP_TSC_DEADLINE_TIMER: usize = 72;
pub const KVM_CAP_SIGNAL_MSI: usize = 77;
pub const KVM_CAP_READONLY_MEM: usize = 81;

pub const KVM_CAP_ENABLE_CAP_VM: usize = 98;
pub const KVM_CAP_SPLIT_IRQCHIP: usize = 121;
pub const KVM_CAP_IOEVENTFD_ANY_LENGTH: usize = 122;
pub const KVM_CAP_MAX_VCPU_ID: usize = 128;

pub const KVM_CAP_COALESCED_PIO: usize = 162;
pub const KVM_CAP_DIRTY_LOG_RING: usize = 192;
pub const KVM_CAP_VM_GPA_BITS: usize = 207;
pub const KVM_CAP_DIRTY_LOG_RING_ACQ_REL: usize = 223;
pub const KVM_CAP_RISCV_MP_STATE_RESET: usize = 242;

pub const KVM_IRQCHIP_PIC_MASTER: u32 = 0;
pub const KVM_IRQCHIP_PIC_SLAVE: u32 = 1;
pub const KVM_IRQCHIP_IOAPIC: u32 = 2;
pub const KVM_IRQ_ROUTING_IRQCHIP: u32 = 1;
pub const KVM_IRQ_ROUTING_MSI: u32 = 2;
pub const KVM_IOEVENTFD_FLAG_DATAMATCH: u32 = 1 << 0;
pub const KVM_IOEVENTFD_FLAG_PIO: u32 = 1 << 1;
pub const KVM_IOEVENTFD_FLAG_DEASSIGN: u32 = 1 << 2;
pub const KVM_IRQFD_FLAG_DEASSIGN: u32 = 1 << 0;
pub const KVM_IRQFD_FLAG_RESAMPLE: u32 = 1 << 1;
pub const KVM_MAX_IRQ_ROUTES: usize = 4096;
pub const KVM_MAX_NR_MEMSLOTS: i32 = 32;

pub const KVM_MP_STATE_RUNNABLE: u32 = 0;
pub const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
pub const KVM_MP_STATE_INIT_RECEIVED: u32 = 2;
pub const KVM_MP_STATE_HALTED: u32 = 3;

pub const KVM_RUN_STRUCT_SIZE: usize = 2352;

pub const KVM_RUN_IMMEDIATE_EXIT_OFFSET: usize = 1;
pub const KVM_RUN_EXIT_REASON_OFFSET: usize = 8;
pub const KVM_RUN_READY_FOR_INTERRUPT_INJECTION_OFFSET: usize = 12;
pub const KVM_RUN_IF_FLAG_OFFSET: usize = 13;
pub const KVM_RUN_FLAGS_OFFSET: usize = 14;
pub const KVM_RUN_CR8_OFFSET: usize = 16;
pub const KVM_RUN_APIC_BASE_OFFSET: usize = 24;

pub const KVM_RUN_IO_DIRECTION_OFFSET: usize = 32;
pub const KVM_RUN_IO_SIZE_OFFSET: usize = 33;
pub const KVM_RUN_IO_PORT_OFFSET: usize = 34;
pub const KVM_RUN_IO_COUNT_OFFSET: usize = 36;
pub const KVM_RUN_IO_DATA_OFFSET_OFFSET: usize = 40;
pub const KVM_RUN_IO_DATA_OFFSET: usize = 2560;
pub const KVM_RUN_IO_DATA_CAPACITY: usize = PAGE_SIZE;

pub const KVM_RUN_MMIO_PHYS_ADDR_OFFSET: usize = 32;
pub const KVM_RUN_MMIO_DATA_OFFSET: usize = 40;
pub const KVM_RUN_MMIO_LEN_OFFSET: usize = 48;
pub const KVM_RUN_MMIO_IS_WRITE_OFFSET: usize = 52;

pub const KVM_EXIT_UNKNOWN: u32 = 0;
pub const KVM_EXIT_IO: u32 = 2;
pub const KVM_EXIT_DEBUG: u32 = 2;
pub const KVM_EXIT_HLT: u32 = 5;
pub const KVM_EXIT_MMIO: u32 = 6;
pub const KVM_EXIT_SHUTDOWN: u32 = 8;
pub const KVM_EXIT_INTERNAL_ERROR: u32 = 17;
pub const KVM_EXIT_RISCV_SBI: u32 = 35;
pub const KVM_EXIT_RISCV_CSR: u32 = 36;

pub const KVM_EXIT_IO_IN: u8 = 0;
pub const KVM_EXIT_IO_OUT: u8 = 1;

pub const IA32_TSC_DEADLINE: u32 = 0x6e0;

/* Exception causes */
pub const EXC_INST_MISALIGNED: usize = 0;
pub const EXC_INST_ACCESS: usize = 1;
pub const EXC_INST_ILLEGAL: usize = 2;
pub const EXC_BREAKPOINT: usize = 3;
pub const EXC_LOAD_MISALIGNED: usize = 4;
pub const EXC_LOAD_ACCESS: usize = 5;
pub const EXC_STORE_MISALIGNED: usize = 6;
pub const EXC_STORE_ACCESS: usize = 7;
pub const EXC_SYSCALL: usize = 8;
pub const EXC_HYPERVISOR_SYSCALL: usize = 9;
pub const EXC_SUPERVISOR_SYSCALL: usize = 10;
pub const EXC_INST_PAGE_FAULT: usize = 12;
pub const EXC_LOAD_PAGE_FAULT: usize = 13;
pub const EXC_STORE_PAGE_FAULT: usize = 15;
pub const EXC_INST_GUEST_PAGE_FAULT: usize = 20;
pub const EXC_LOAD_GUEST_PAGE_FAULT: usize = 21;
pub const EXC_VIRTUAL_INST_FAULT: usize = 22;
pub const EXC_STORE_GUEST_PAGE_FAULT: usize = 23;

pub const INSN_OPCODE_SYSTEM: usize = 28;

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

        _ => check_arch_extension(raw_ioctl),
    }
}

pub fn read_vcpu_id(raw_ioctl: RawIoctl) -> Result<u32> {
    Ok(u32::try_from(raw_ioctl.arg())?)
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
