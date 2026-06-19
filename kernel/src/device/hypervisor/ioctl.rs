//! Ioctl api compatible with Linux KVM.
//! KVM api: https://www.kernel.org/doc/html/latest/virt/kvm/api.html

use crate::{
    prelude::*,
    util::ioctl::{InData, NoData, OutData, ioc},
};

const KVM_INTERRUPT_BITMAP_WORDS: usize = (256 + 63) / 64;
pub(super) const KVM_MEM_READONLY: u32 = 1 << 1;

pub(super) const KVM_API_VERSION: i32 = 12;
pub(super) const KVM_RUN_MMAP_SIZE: usize = PAGE_SIZE;
pub(super) const KVM_RUN_STRUCT_SIZE: usize = 2352;
const KVM_RUN_EXIT_DATA_OFFSET: usize = 32;
const KVM_RUN_EXIT_DATA_SIZE: usize = KVM_RUN_STRUCT_SIZE - KVM_RUN_EXIT_DATA_OFFSET;

pub(super) const KVM_RUN_IMMEDIATE_EXIT_OFFSET: usize = 1;
pub(super) const KVM_RUN_EXIT_REASON_OFFSET: usize = 8;
pub(super) const KVM_RUN_READY_FOR_INTERRUPT_INJECTION_OFFSET: usize = 12;
pub(super) const KVM_RUN_IF_FLAG_OFFSET: usize = 13;
pub(super) const KVM_RUN_FLAGS_OFFSET: usize = 14;
pub(super) const KVM_RUN_CR8_OFFSET: usize = 16;
pub(super) const KVM_RUN_APIC_BASE_OFFSET: usize = 24;

pub(super) const KVM_RUN_IO_DIRECTION_OFFSET: usize = 32;
pub(super) const KVM_RUN_IO_SIZE_OFFSET: usize = 33;
pub(super) const KVM_RUN_IO_PORT_OFFSET: usize = 34;
pub(super) const KVM_RUN_IO_COUNT_OFFSET: usize = 36;
pub(super) const KVM_RUN_IO_DATA_OFFSET_OFFSET: usize = 40;
pub(super) const KVM_RUN_IO_DATA_OFFSET: usize = 2560;

pub(super) const KVM_RUN_MMIO_PHYS_ADDR_OFFSET: usize = 32;
pub(super) const KVM_RUN_MMIO_DATA_OFFSET: usize = 40;
pub(super) const KVM_RUN_MMIO_LEN_OFFSET: usize = 48;
pub(super) const KVM_RUN_MMIO_IS_WRITE_OFFSET: usize = 52;

pub(super) const KVM_EXIT_IO: u32 = 2;
pub(super) const KVM_EXIT_HLT: u32 = 5;
pub(super) const KVM_EXIT_MMIO: u32 = 6;
pub(super) const KVM_EXIT_INTR: u32 = 10;
pub(super) const KVM_EXIT_INTERNAL_ERROR: u32 = 17;

pub(super) const KVM_EXIT_IO_IN: u8 = 0;
pub(super) const KVM_EXIT_IO_OUT: u8 = 1;

// KVM _IO commands may still pass scalar values in the ioctl argument.
// The command word itself encodes no direction or data size for them.

// System ioctls.
pub(super) type GetApiVersion = ioc!(KVM_GET_API_VERSION, 0xAE, 0x00, NoData);
pub(super) type CreateVm = ioc!(KVM_CREATE_VM, 0xAE, 0x01, NoData);
// pub(super) type CheckExtension = ioc!(KVM_CHECK_EXTENSION, 0xAE, 0x03, NoData);
pub(super) type GetVcpuMmapSize = ioc!(KVM_GET_VCPU_MMAP_SIZE, 0xAE, 0x04, NoData);

// VM ioctls.
pub(super) type CreateVcpu = ioc!(KVM_CREATE_VCPU, 0xAE, 0x41, NoData);
pub(super) type SetUserMemoryRegion = ioc!(
    KVM_SET_USER_MEMORY_REGION,
    0xAE,
    0x46,
    InData<UserMemoryRegion>
);
pub(super) type SetTssAddr = ioc!(KVM_SET_TSS_ADDR, 0xAE, 0x47, NoData);

// VCPU ioctls.
pub(super) type Run = ioc!(KVM_RUN, 0xAE, 0x80, NoData);
pub(super) type GetRegs = ioc!(KVM_GET_REGS, 0xAE, 0x81, OutData<VcpuRegs>);
pub(super) type SetRegs = ioc!(KVM_SET_REGS, 0xAE, 0x82, InData<VcpuRegs>);
pub(super) type GetSregs = ioc!(KVM_GET_SREGS, 0xAE, 0x83, OutData<VcpuSregs>);
pub(super) type SetSregs = ioc!(KVM_SET_SREGS, 0xAE, 0x84, InData<VcpuSregs>);

/// The x86 `struct kvm_userspace_memory_region`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct UserMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

/// The x86 `struct kvm_regs`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

/// The x86 `struct kvm_segment`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

/// The x86 `struct kvm_dtable`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

/// The x86 `struct kvm_sregs`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuSregs {
    pub cs: VcpuSegment,
    pub ds: VcpuSegment,
    pub es: VcpuSegment,
    pub fs: VcpuSegment,
    pub gs: VcpuSegment,
    pub ss: VcpuSegment,
    pub tr: VcpuSegment,
    pub ldt: VcpuSegment,
    pub gdt: VcpuDtable,
    pub idt: VcpuDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; KVM_INTERRUPT_BITMAP_WORDS],
}

/// The x86 `struct kvm_run`.
///
/// The Linux layout contains a large union starting at byte 32. Kernel code
/// writes fields by offset so this definition can stay safe Rust while still
/// documenting the userspace ABI shape.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct KvmRun {
    pub request_interrupt_window: u8,
    pub immediate_exit: u8,
    pub padding1: [u8; 6],
    pub exit_reason: u32,
    pub ready_for_interrupt_injection: u8,
    pub if_flag: u8,
    pub flags: u16,
    pub cr8: u64,
    pub apic_base: u64,
    pub exit_data: [u8; KVM_RUN_EXIT_DATA_SIZE],
}

const _: () = assert!(core::mem::size_of::<KvmRun>() == KVM_RUN_STRUCT_SIZE);
