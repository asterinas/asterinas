use core::arch::x86_64::CpuidResult;

use crate::{
    arch::{cpu::cpuid::cpuid, tsc_freq, vm::context::GuestContext},
    prelude::*,
    sync::Mutex,
};

pub(crate) fn emulate_cpuid(context: &Mutex<GuestContext>) -> Result<()> {
    const CPUID_1_ECX_VMX: u32 = 1 << 5;
    const CPUID_1_ECX_FMA: u32 = 1 << 12;
    const CPUID_1_ECX_X2APIC: u32 = 1 << 21;
    const CPUID_1_ECX_TSC_DEADLINE: u32 = 1 << 24;
    const CPUID_1_ECX_PCID: u32 = 1 << 17;
    const CPUID_1_ECX_XSAVE: u32 = 1 << 26;
    const CPUID_1_ECX_OSXSAVE: u32 = 1 << 27;
    const CPUID_1_ECX_AVX: u32 = 1 << 28;
    const CPUID_1_EDX_APIC: u32 = 1 << 9;
    const CPUID_1_EDX_HTT: u32 = 1 << 28;
    const CPUID_7_EBX_FSGSBASE: u32 = 1 << 0;
    const CPUID_7_EBX_HLE: u32 = 1 << 4;
    const CPUID_7_EBX_AVX2: u32 = 1 << 5;
    const CPUID_7_EBX_RTM: u32 = 1 << 11;
    const CPUID_7_EBX_INVPCID: u32 = 1 << 10;
    const CPUID_7_EBX_AVX512F: u32 = 1 << 16;
    const CPUID_7_EBX_AVX512DQ: u32 = 1 << 17;
    const CPUID_7_EBX_AVX512CD: u32 = 1 << 28;
    const CPUID_7_EBX_AVX512BW: u32 = 1 << 30;
    const CPUID_7_EBX_AVX512VL: u32 = 1 << 31;
    const CPUID_7_ECX_AVX512VBMI: u32 = 1 << 1;
    const CPUID_7_ECX_VAES: u32 = 1 << 9;
    const CPUID_7_ECX_VPCLMULQDQ: u32 = 1 << 10;
    const CPUID_7_ECX_AVX512VNNI: u32 = 1 << 11;
    const CPUID_7_ECX_AVX512BITALG: u32 = 1 << 12;
    const CPUID_7_ECX_AVX512VPOPCNTDQ: u32 = 1 << 14;

    let vcpu_count = context.lock().cpu_config.vcpu_count;
    let apic_id = context.lock().cpu_config.lapic_id;

    let eax = context.lock().arch().gpr(0) as u32;
    let ecx = context.lock().arch().gpr(2) as u32;
    let (mut eax_out, mut ebx_out, mut ecx_out, mut edx_out) =
        if let Some(CpuidResult { eax, ebx, ecx, edx }) = cpuid(eax, ecx) {
            (eax, ebx, ecx, edx)
        } else {
            (0, 0, 0, 0)
        };

    if eax == 0 {
        eax_out = eax_out.max(0x16);
    }

    if eax == 1 {
        ecx_out &= !(CPUID_1_ECX_VMX
            | CPUID_1_ECX_FMA
            | CPUID_1_ECX_X2APIC
            | CPUID_1_ECX_TSC_DEADLINE
            | CPUID_1_ECX_PCID
            | CPUID_1_ECX_XSAVE
            | CPUID_1_ECX_OSXSAVE
            | CPUID_1_ECX_AVX);
        ebx_out = (ebx_out & 0x0000_ffff) | ((vcpu_count & 0xff) << 16) | ((apic_id & 0xff) << 24);
        edx_out |= CPUID_1_EDX_APIC;
        if vcpu_count > 1 {
            edx_out |= CPUID_1_EDX_HTT;
        } else {
            edx_out &= !CPUID_1_EDX_HTT;
        }
    }

    if eax == 4 {
        if (eax_out & 0x1f) != 0 {
            let cores_per_package_minus_one = vcpu_count.saturating_sub(1).min(0x3f);
            eax_out = (eax_out & !(0x3f << 26)) | (cores_per_package_minus_one << 26);
        }
    }

    if eax == 7 && ecx == 0 {
        ebx_out &= !(CPUID_7_EBX_FSGSBASE
            | CPUID_7_EBX_HLE
            | CPUID_7_EBX_AVX2
            | CPUID_7_EBX_RTM
            | CPUID_7_EBX_INVPCID
            | CPUID_7_EBX_AVX512F
            | CPUID_7_EBX_AVX512DQ
            | CPUID_7_EBX_AVX512CD
            | CPUID_7_EBX_AVX512BW
            | CPUID_7_EBX_AVX512VL);
        ecx_out &= !(CPUID_7_ECX_AVX512VBMI
            | CPUID_7_ECX_VAES
            | CPUID_7_ECX_VPCLMULQDQ
            | CPUID_7_ECX_AVX512VNNI
            | CPUID_7_ECX_AVX512BITALG
            | CPUID_7_ECX_AVX512VPOPCNTDQ);
    }

    if eax == 0xd {
        eax_out = 0;
        ebx_out = 0;
        ecx_out = 0;
        edx_out = 0;
    }

    if eax == 0x0b || eax == 0x1f {
        let topology = topology_cpuid(ecx, apic_id, vcpu_count);
        eax_out = topology.eax;
        ebx_out = topology.ebx;
        ecx_out = topology.ecx;
        edx_out = topology.edx;
    }

    const CPUID_TSC_CRYSTAL_HZ: u32 = 1_000_000;
    if eax == 0x15 {
        if let Some(tsc_mhz) = virtual_tsc_mhz() {
            eax_out = 1;
            ebx_out = tsc_mhz;
            ecx_out = CPUID_TSC_CRYSTAL_HZ;
            edx_out = 0;
        }
    }

    if eax == 0x16 {
        if let Some(tsc_mhz) = virtual_tsc_mhz() {
            eax_out = tsc_mhz;
            ebx_out = tsc_mhz;
            ecx_out = 0;
            edx_out = 0;
        }
    }

    context.lock().arch_mut().set_gpr(0, 8, eax_out as u64);
    context.lock().arch_mut().set_gpr(1, 8, ebx_out as u64);
    context.lock().arch_mut().set_gpr(2, 8, ecx_out as u64);
    context.lock().arch_mut().set_gpr(3, 8, edx_out as u64);

    Ok(())
}

fn topology_cpuid(subleaf: u32, apic_id: u32, vcpu_count: u32) -> CpuidResult {
    if vcpu_count <= 1 {
        return CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: subleaf,
            edx: apic_id,
        };
    }

    match subleaf {
        0 => CpuidResult {
            // One hardware thread per guest core. Keep the SMT level present
            // but sized to 1 so Linux models --vcpus as separate cores.
            eax: 0,
            ebx: 1,
            ecx: 1 << 8,
            edx: apic_id,
        },
        1 => CpuidResult {
            eax: topology_apic_id_shift(vcpu_count),
            ebx: vcpu_count,
            ecx: (2 << 8) | 1,
            edx: apic_id,
        },
        _ => CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: subleaf,
            edx: apic_id,
        },
    }
}

fn topology_apic_id_shift(vcpu_count: u32) -> u32 {
    u32::BITS - vcpu_count.saturating_sub(1).leading_zeros()
}

fn virtual_tsc_mhz() -> Option<u32> {
    let mhz = (tsc_freq().saturating_add(500_000)) / 1_000_000;
    u32::try_from(mhz).ok().filter(|&mhz| mhz != 0)
}
