use crate::arch::cpu::extension::*;

pub(super) const KVM_RISCV_BASE_ISA_MASK: usize = 0x0000000003FFFFFF;

/// ISA extension IDs specific to KVM. This is not the same as the host ISA
/// extension IDs as that is internal to the host and should not be exposed
/// to the guest. This should always be contiguous to keep the mapping simple
/// in KVM implementation.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum KvmRiscvIsaExtId {
    A = 0,
    C,
    D,
    F,
    H,
    I,
    M,
    SVPBMT,
    SSTC,
    SVINVAL,
    ZIHINTPAUSE,
    ZICBOM,
    ZICBOZ,
    ZBB,
    SSAIA,
    V,
    SVNAPOT,
    ZBA,
    ZBS,
    ZICNTR,
    ZICSR,
    ZIFENCEI,
    ZIHPM,
    SMSTATEEN,
    ZICOND,
    ZBC,
    ZBKB,
    ZBKC,
    ZBKX,
    ZKND,
    ZKNE,
    ZKNH,
    ZKR,
    ZKSED,
    ZKSH,
    ZKT,
    ZVBB,
    ZVBC,
    ZVKB,
    ZVKG,
    ZVKNED,
    ZVKNHA,
    ZVKNHB,
    ZVKSED,
    ZVKSH,
    ZVKT,
    ZFH,
    ZFHMIN,
    ZIHINTNTL,
    ZVFH,
    ZVFHMIN,
    ZFA,
    ZTSO,
    ZACAS,
    SSCOFPMF,
    ZIMOP,
    ZCA,
    ZCB,
    ZCD,
    ZCF,
    ZCMOP,
    ZAWRS,
    SMNPM,
    SSNPM,
    SVADE,
    SVADU,
    SVVPTC,
    ZABHA,
    ZICCRSE,
    ZAAMO,
    ZALRSC,
    ZICBOP,
    ZFBFMIN,
    ZVFBFMIN,
    ZVFBFWMA,
    ZCLSD,
    ZILSD,
    ZALASR,
    MAX,
}

/// 1
static KVM_ISA_EXT_ARR: [IsaExtensions; KvmRiscvIsaExtId::MAX as usize] = [
    // single
    IsaExtensions::A,          // KVM_RISCV_ISA_EXT_A
    IsaExtensions::C,          // KVM_RISCV_ISA_EXT_C
    IsaExtensions::D,          // KVM_RISCV_ISA_EXT_D
    IsaExtensions::F,          // KVM_RISCV_ISA_EXT_F
    IsaExtensions::H,          // KVM_RISCV_ISA_EXT_H
    IsaExtensions::I,          // KVM_RISCV_ISA_EXT_I
    IsaExtensions::M,          // KVM_RISCV_ISA_EXT_M
    IsaExtensions::V,          // KVM_RISCV_ISA_EXT_V

    // multi
    IsaExtensions::SMNPM,      // KVM_RISCV_ISA_EXT_SMNPM
    IsaExtensions::SMSTATEEN,  // KVM_RISCV_ISA_EXT_SMSTATEEN
    IsaExtensions::SSAIA,      // KVM_RISCV_ISA_EXT_SSAIA
    IsaExtensions::SSCOFPMF,   // KVM_RISCV_ISA_EXT_SSCOFPMF
    IsaExtensions::SSNPM,      // KVM_RISCV_ISA_EXT_SSNPM
    IsaExtensions::SSTC,       // KVM_RISCV_ISA_EXT_SSTC
    IsaExtensions::SVADE,      // KVM_RISCV_ISA_EXT_SVADE
    IsaExtensions::SVADU,      // KVM_RISCV_ISA_EXT_SVADU
    IsaExtensions::SVINVAL,    // KVM_RISCV_ISA_EXT_SVINVAL
    IsaExtensions::SVNAPOT,    // KVM_RISCV_ISA_EXT_SVNAPOT
    IsaExtensions::SVPBMT,     // KVM_RISCV_ISA_EXT_SVPBMT
    IsaExtensions::SVVPTC,     // KVM_RISCV_ISA_EXT_SVVPTC
    IsaExtensions::ZAAMO,      // KVM_RISCV_ISA_EXT_ZAAMO
    IsaExtensions::ZABHA,      // KVM_RISCV_ISA_EXT_ZABHA
    IsaExtensions::ZACAS,      // KVM_RISCV_ISA_EXT_ZACAS
    IsaExtensions::ZALASR,     // KVM_RISCV_ISA_EXT_ZALASR
    IsaExtensions::ZALRSC,     // KVM_RISCV_ISA_EXT_ZALRSC
    IsaExtensions::ZAWRS,      // KVM_RISCV_ISA_EXT_ZAWRS
    IsaExtensions::ZBA,        // KVM_RISCV_ISA_EXT_ZBA
    IsaExtensions::ZBB,        // KVM_RISCV_ISA_EXT_ZBB
    IsaExtensions::ZBC,        // KVM_RISCV_ISA_EXT_ZBC
    IsaExtensions::ZBKB,       // KVM_RISCV_ISA_EXT_ZBKB
    IsaExtensions::ZBKC,       // KVM_RISCV_ISA_EXT_ZBKC
    IsaExtensions::ZBKX,       // KVM_RISCV_ISA_EXT_ZBKX
    IsaExtensions::ZBS,        // KVM_RISCV_ISA_EXT_ZBS
    IsaExtensions::ZCA,        // KVM_RISCV_ISA_EXT_ZCA
    IsaExtensions::ZCB,        // KVM_RISCV_ISA_EXT_ZCB
    IsaExtensions::ZCD,        // KVM_RISCV_ISA_EXT_ZCD
    IsaExtensions::ZCF,        // KVM_RISCV_ISA_EXT_ZCF
    IsaExtensions::ZCLSD,      // KVM_RISCV_ISA_EXT_ZCLSD
    IsaExtensions::ZCMOP,      // KVM_RISCV_ISA_EXT_ZCMOP
    IsaExtensions::ZFA,        // KVM_RISCV_ISA_EXT_ZFA
    IsaExtensions::ZFBFMIN,    // KVM_RISCV_ISA_EXT_ZFBFMIN
    IsaExtensions::ZFH,        // KVM_RISCV_ISA_EXT_ZFH
    IsaExtensions::ZFHMIN,     // KVM_RISCV_ISA_EXT_ZFHMIN
    IsaExtensions::ZICBOM,     // KVM_RISCV_ISA_EXT_ZICBOM
    IsaExtensions::ZICBOP,     // KVM_RISCV_ISA_EXT_ZICBOP
    IsaExtensions::ZICBOZ,     // KVM_RISCV_ISA_EXT_ZICBOZ
    IsaExtensions::ZICCRSE,    // KVM_RISCV_ISA_EXT_ZICCRSE
    IsaExtensions::ZICNTR,     // KVM_RISCV_ISA_EXT_ZICNTR
    IsaExtensions::ZICOND,     // KVM_RISCV_ISA_EXT_ZICOND
    IsaExtensions::ZICSR,      // KVM_RISCV_ISA_EXT_ZICSR
    IsaExtensions::ZIFENCEI,   // KVM_RISCV_ISA_EXT_ZIFENCEI
    IsaExtensions::ZIHINTNTL,  // KVM_RISCV_ISA_EXT_ZIHINTNTL
    IsaExtensions::ZIHINTPAUSE,// KVM_RISCV_ISA_EXT_ZIHINTPAUSE
    IsaExtensions::ZIHPM,      // KVM_RISCV_ISA_EXT_ZIHPM
    IsaExtensions::ZILSD,      // KVM_RISCV_ISA_EXT_ZILSD
    IsaExtensions::ZIMOP,      // KVM_RISCV_ISA_EXT_ZIMOP
    IsaExtensions::ZKND,       // KVM_RISCV_ISA_EXT_ZKND
    IsaExtensions::ZKNE,       // KVM_RISCV_ISA_EXT_ZKNE
    IsaExtensions::ZKNH,       // KVM_RISCV_ISA_EXT_ZKNH
    IsaExtensions::ZKR,        // KVM_RISCV_ISA_EXT_ZKR
    IsaExtensions::ZKSED,      // KVM_RISCV_ISA_EXT_ZKSED
    IsaExtensions::ZKSH,       // KVM_RISCV_ISA_EXT_ZKSH
    IsaExtensions::ZKT,        // KVM_RISCV_ISA_EXT_ZKT
    IsaExtensions::ZTSO,       // KVM_RISCV_ISA_EXT_ZTSO
    IsaExtensions::ZVBB,       // KVM_RISCV_ISA_EXT_ZVBB
    IsaExtensions::ZVBC,       // KVM_RISCV_ISA_EXT_ZVBC
    IsaExtensions::ZVFBFMIN,   // KVM_RISCV_ISA_EXT_ZVFBFMIN
    IsaExtensions::ZVFBFWMA,   // KVM_RISCV_ISA_EXT_ZVFBFWMA
    IsaExtensions::ZVFH,       // KVM_RISCV_ISA_EXT_ZVFH
    IsaExtensions::ZVFHMIN,    // KVM_RISCV_ISA_EXT_ZVFHMIN
    IsaExtensions::ZVKB,       // KVM_RISCV_ISA_EXT_ZVKB
    IsaExtensions::ZVKG,       // KVM_RISCV_ISA_EXT_ZVKG
    IsaExtensions::ZVKNED,     // KVM_RISCV_ISA_EXT_ZVKNED
    IsaExtensions::ZVKNHA,     // KVM_RISCV_ISA_EXT_ZVKNHA
    IsaExtensions::ZVKNHB,     // KVM_RISCV_ISA_EXT_ZVKNHB
    IsaExtensions::ZVKSED,     // KVM_RISCV_ISA_EXT_ZVKSED
    IsaExtensions::ZVKSH,      // KVM_RISCV_ISA_EXT_ZVKSH
    IsaExtensions::ZVKT,       // KVM_RISCV_ISA_EXT_ZVKT
];

/// 1
fn kvm_riscv_base2isa_ext(base_ext: IsaExtensions) -> usize {
    for i in 0..KvmRiscvIsaExtId::MAX as usize {
        if KVM_ISA_EXT_ARR[i] == base_ext {
            return i;
        }
    }
    KvmRiscvIsaExtId::MAX as usize
}

/// 1
fn kvm_riscv_isa_check_host(kvm_ext: usize) -> Result<IsaExtensions, i32> {
    let mapped = KVM_ISA_EXT_ARR[kvm_ext];

    let host_ext = match mapped {
        IsaExtensions::SMNPM => IsaExtensions::SSNPM,
        other => other,
    };

    if !has_extensions(host_ext) {
        return Err(1);
    }

    Ok(mapped)
}

/// 判断某个 KVM ISA 扩展是否允许对 guest 启用。
fn kvm_riscv_isa_enable_allowed(ext: IsaExtensions) -> bool {
    match ext {
        IsaExtensions::H | IsaExtensions::SSCOFPMF |IsaExtensions::V => false,
        _ => has_extensions(ext),
    }
}

/// 判断某个 KVM ISA 扩展是否允许对 guest 禁用。
fn kvm_riscv_isa_disable_allowed(_ext: usize) -> bool {
    false
}

fn set_bit(isa: &mut [u64; 2], bit: usize) {
    if bit < 64 {
        isa[0] |= 1<<bit
    } else {
        isa[1] |= 1<<(bit-64)
    }
}

/// 1
pub(super) fn kvm_riscv_vcpu_setup_isa(isa: &mut [u64; 2]) {
    for i in 0..KvmRiscvIsaExtId::MAX as usize {
        if let Ok(ext) = kvm_riscv_isa_check_host(i) {
            if kvm_riscv_isa_enable_allowed(ext) {
                set_bit(isa, i);
            }
        }
    }
}
