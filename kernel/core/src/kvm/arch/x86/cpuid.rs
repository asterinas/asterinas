use ostd::arch::cpu::cpuid;

use super::ioctl::KvmCpuidEntry2;

// `CPUID.01H:EDX`.
const CPUID_PSE: u32 = 1 << 3;
const CPUID_PAE: u32 = 1 << 6;

// `CPUID.01H:ECX`.
const CPUID_HYPERVISOR: u32 = 1 << 31;

// `CPUID.80000001H:EDX`.
pub(super) const CPUID_LM: u32 = 1 << 29;

pub(super) fn supported_cpuid() -> [KvmCpuidEntry2; 7] {
    const BASIC_EDX: u32 = CPUID_PSE | CPUID_PAE;

    const EXTENDED_EDX: u32 = CPUID_LM;

    let vendor = cpuid::cpuid(0, 0).unwrap();
    let basic = cpuid::cpuid(1, 0).unwrap();
    let extended = cpuid::cpuid(0x8000_0001, 0).unwrap();

    [
        // Maximum basic CPUID leaf and CPU vendor ID.
        KvmCpuidEntry2 {
            function: 0,
            eax: 1,
            ebx: vendor.ebx,
            ecx: vendor.ecx,
            edx: vendor.edx,
            ..Default::default()
        },
        // Processor signature, initial APIC ID, and basic feature bits.
        KvmCpuidEntry2 {
            function: 1,
            eax: basic.eax,
            // One logical processor.
            ebx: 1 << 16,
            ecx: CPUID_HYPERVISOR,
            edx: basic.edx & BASIC_EDX,
            ..Default::default()
        },
        // KVM hypervisor signature and maximum hypervisor CPUID leaf.
        KvmCpuidEntry2 {
            function: 0x4000_0000,
            eax: 0x4000_0001,
            ebx: u32::from_le_bytes(*b"KVMK"),
            ecx: u32::from_le_bytes(*b"VMKV"),
            edx: u32::from_le_bytes(*b"M\0\0\0"),
            ..Default::default()
        },
        // KVM paravirtual feature bits.
        KvmCpuidEntry2 {
            function: 0x4000_0001,
            ..Default::default()
        },
        // Maximum extended CPUID leaf.
        KvmCpuidEntry2 {
            function: 0x8000_0000,
            eax: 0x8000_0008,
            ..Default::default()
        },
        // Extended processor feature bits, including NX and long mode.
        KvmCpuidEntry2 {
            function: 0x8000_0001,
            ecx: extended.ecx,
            edx: extended.edx & EXTENDED_EDX,
            ..Default::default()
        },
        // Physical and linear address widths.
        KvmCpuidEntry2 {
            function: 0x8000_0008,
            eax: 36 | (48 << 8),
            ..Default::default()
        },
    ]
}
