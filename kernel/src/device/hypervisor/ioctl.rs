//! Ioctl api compatible with Linux KVM.
//! KVM api: https://www.kernel.org/doc/html/latest/virt/kvm/api.html

use ostd::arch::vm::GuestCpuidEntry;

use crate::{
    prelude::*,
    util::ioctl::{InData, InOutData, NoData, OutData, ioc},
};

const KVM_INTERRUPT_BITMAP_WORDS: usize = (256 + 63) / 64;
const KVM_APIC_REG_SIZE: usize = 0x400;
pub(super) const KVM_MEM_READONLY: u32 = 1 << 1;

pub(super) const KVM_API_VERSION: i32 = 12;
pub(super) const KVM_RECOMMENDED_VCPUS: i32 = 1;
pub(super) const KVM_MAX_VCPUS: i32 = 64;
pub(super) const KVM_MAX_CPUID_ENTRIES: usize = 100;
pub(super) const KVM_MAX_MSR_ENTRIES: usize = 100;

pub(super) const KVM_CAP_IRQCHIP: usize = 0;
pub(super) const KVM_CAP_HLT: usize = 1;
pub(super) const KVM_CAP_USER_MEMORY: usize = 3;
pub(super) const KVM_CAP_SET_TSS_ADDR: usize = 4;
pub(super) const KVM_CAP_EXT_CPUID: usize = 7;
pub(super) const KVM_CAP_NR_VCPUS: usize = 9;
pub(super) const KVM_CAP_COALESCED_MMIO: usize = 15;
pub(super) const KVM_CAP_IRQ_ROUTING: usize = 25;
pub(super) const KVM_CAP_IRQ_INJECT_STATUS: usize = 26;
pub(super) const KVM_CAP_PIT2: usize = 33;
pub(super) const KVM_CAP_MAX_VCPUS: usize = 66;

pub(super) const KVM_IRQCHIP_IOAPIC: u32 = 2;
pub(super) const KVM_IRQ_ROUTING_IRQCHIP: u32 = 1;
pub(super) const KVM_IRQ_ROUTING_MSI: u32 = 2;
pub(super) const KVM_MAX_IRQ_ROUTES: usize = 4096;

pub(super) const KVM_COALESCED_MMIO_PAGE_OFFSET: usize = 2;
pub(super) const KVM_RUN_MMAP_SIZE: usize = (KVM_COALESCED_MMIO_PAGE_OFFSET + 1) * PAGE_SIZE;
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
pub(super) type CheckExtension = ioc!(KVM_CHECK_EXTENSION, 0xAE, 0x03, NoData);
pub(super) type GetVcpuMmapSize = ioc!(KVM_GET_VCPU_MMAP_SIZE, 0xAE, 0x04, NoData);
pub(super) type GetSupportedCpuid =
    ioc!(KVM_GET_SUPPORTED_CPUID, 0xAE, 0x05, InOutData<VcpuCpuid2>);

// VM ioctls.
pub(super) type CreateVcpu = ioc!(KVM_CREATE_VCPU, 0xAE, 0x41, NoData);
pub(super) type SetUserMemoryRegion = ioc!(
    KVM_SET_USER_MEMORY_REGION,
    0xAE,
    0x46,
    InData<UserMemoryRegion>
);
pub(super) type SetTssAddr = ioc!(KVM_SET_TSS_ADDR, 0xAE, 0x47, NoData);
pub(super) type CreateIrqchip = ioc!(KVM_CREATE_IRQCHIP, 0xAE, 0x60, NoData);
pub(super) type IrqLine = ioc!(KVM_IRQ_LINE, 0xAE, 0x61, InData<IrqLevel>);
pub(super) type RegisterCoalescedMmio =
    ioc!(KVM_REGISTER_COALESCED_MMIO, 0xAE, 0x67, InData<CoalescedMmioZone>);
pub(super) type UnregisterCoalescedMmio = ioc!(
    KVM_UNREGISTER_COALESCED_MMIO,
    0xAE,
    0x68,
    InData<CoalescedMmioZone>
);
pub(super) type SetGsiRouting = ioc!(KVM_SET_GSI_ROUTING, 0xAE, 0x6a, InData<IrqRouting>);
pub(super) type CreatePit2 = ioc!(KVM_CREATE_PIT2, 0xAE, 0x77, InData<PitConfig>);

// VCPU ioctls.
pub(super) type Run = ioc!(KVM_RUN, 0xAE, 0x80, NoData);
pub(super) type GetRegs = ioc!(KVM_GET_REGS, 0xAE, 0x81, OutData<VcpuRegs>);
pub(super) type SetRegs = ioc!(KVM_SET_REGS, 0xAE, 0x82, InData<VcpuRegs>);
pub(super) type GetSregs = ioc!(KVM_GET_SREGS, 0xAE, 0x83, OutData<VcpuSregs>);
pub(super) type SetSregs = ioc!(KVM_SET_SREGS, 0xAE, 0x84, InData<VcpuSregs>);
pub(super) type GetMsrs = ioc!(KVM_GET_MSRS, 0xAE, 0x88, InOutData<VcpuMsrs>);
pub(super) type SetMsrs = ioc!(KVM_SET_MSRS, 0xAE, 0x89, InData<VcpuMsrs>);
pub(super) type SetFpu = ioc!(KVM_SET_FPU, 0xAE, 0x8d, InData<VcpuFpu>);
pub(super) type GetLapic = ioc!(KVM_GET_LAPIC, 0xAE, 0x8e, OutData<LapicState>);
pub(super) type SetLapic = ioc!(KVM_SET_LAPIC, 0xAE, 0x8f, InData<LapicState>);
pub(super) type SetCpuid2 = ioc!(KVM_SET_CPUID2, 0xAE, 0x90, InData<VcpuCpuid2>);

pub(super) fn check_extension(extension: usize) -> i32 {
    match extension {
        KVM_CAP_IRQCHIP
        | KVM_CAP_HLT
        | KVM_CAP_USER_MEMORY
        | KVM_CAP_SET_TSS_ADDR
        | KVM_CAP_EXT_CPUID
        | KVM_CAP_IRQ_ROUTING
        | KVM_CAP_IRQ_INJECT_STATUS
        | KVM_CAP_PIT2 => 1,
        KVM_CAP_NR_VCPUS => KVM_RECOMMENDED_VCPUS,
        KVM_CAP_MAX_VCPUS => KVM_MAX_VCPUS,
        KVM_CAP_COALESCED_MMIO => KVM_COALESCED_MMIO_PAGE_OFFSET as i32,
        // TODO: Report capabilities from the actual hypervisor implementation.
        _ => 0,
    }
}

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

/// The common `struct kvm_irq_level`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct IrqLevel {
    pub irq: u32,
    pub level: u32,
}

/// The common `struct kvm_coalesced_mmio_zone`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct CoalescedMmioZone {
    pub addr: u64,
    pub size: u32,
    pub pio: u32,
}

/// The common `struct kvm_pit_config`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct PitConfig {
    pub flags: u32,
    pub pad: [u32; 15],
}

/// The common `struct kvm_irq_routing_entry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct IrqRoutingEntry {
    pub gsi: u32,
    pub type_: u32,
    pub flags: u32,
    pub pad: u32,
    pub data: [u32; 8],
}

/// The common `struct kvm_irq_routing`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct IrqRouting {
    pub nr: u32,
    pub flags: u32,
    pub entries: [IrqRoutingEntry; 0],
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

/// The x86 `struct kvm_lapic_state`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct LapicState {
    pub regs: [u8; KVM_APIC_REG_SIZE],
}

impl Default for LapicState {
    fn default() -> Self {
        Self {
            regs: [0; KVM_APIC_REG_SIZE],
        }
    }
}

/// The x86 `struct kvm_fpu`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuFpu {
    pub fpr: [[u8; 16]; 8],
    pub fcw: u16,
    pub fsw: u16,
    pub ftwx: u8,
    pub pad1: u8,
    pub last_opcode: u16,
    pub last_ip: u64,
    pub last_dp: u64,
    pub xmm: [[u8; 16]; 16],
    pub mxcsr: u32,
    pub pad2: u32,
}

/// The x86 `struct kvm_msr_entry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuMsrEntry {
    pub index: u32,
    pub reserved: u32,
    pub data: u64,
}

/// The x86 `struct kvm_msrs`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuMsrs {
    pub nmsrs: u32,
    pub pad: u32,
    pub entries: [VcpuMsrEntry; 0],
}

/// The x86 `struct kvm_cpuid_entry2`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuCpuidEntry2 {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub padding: [u32; 3],
}

impl From<GuestCpuidEntry> for VcpuCpuidEntry2 {
    fn from(entry: GuestCpuidEntry) -> Self {
        Self {
            function: entry.function,
            index: entry.index,
            flags: entry.flags,
            eax: entry.eax,
            ebx: entry.ebx,
            ecx: entry.ecx,
            edx: entry.edx,
            padding: [0; 3],
        }
    }
}

impl From<VcpuCpuidEntry2> for GuestCpuidEntry {
    fn from(entry: VcpuCpuidEntry2) -> Self {
        Self {
            function: entry.function,
            index: entry.index,
            flags: entry.flags,
            eax: entry.eax,
            ebx: entry.ebx,
            ecx: entry.ecx,
            edx: entry.edx,
        }
    }
}

/// The x86 `struct kvm_cpuid2`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VcpuCpuid2 {
    pub nent: u32,
    pub padding: u32,
    pub entries: [VcpuCpuidEntry2; 0],
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

const _: () = assert!(size_of::<KvmRun>() == KVM_RUN_STRUCT_SIZE);
const _: () = assert!(size_of::<CoalescedMmioZone>() == 16);
const _: () = assert!(size_of::<IrqRouting>() == 8);
const _: () = assert!(size_of::<IrqRoutingEntry>() == 48);
const _: () = assert!(size_of::<VcpuMsrs>() == 8);
const _: () = assert!(size_of::<VcpuCpuid2>() == 8);
const _: () = assert!(size_of::<LapicState>() == KVM_APIC_REG_SIZE);
