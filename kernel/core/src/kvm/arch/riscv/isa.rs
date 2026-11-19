// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;
use ostd::arch::cpu::extension::{IsaExtensions, has_extensions};
use int_to_c_enum::TryFromInt;

pub(super) const KVM_RISCV_BASE_ISA_MASK: usize = 0x0000000003FFFFFF;

/// ISA extension IDs specific to KVM.
/// This is not the same as the host ISA extension IDs,
/// as that is internal to the host and should not be exposed to the guest.
/// This should always be contiguous
/// to keep the mapping simple in the KVM implementation.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TryFromInt)]
pub enum KvmRiscvIsaExtId {
    A = 0,
    C = 1,
    D = 2,
    F = 3,
    H = 4,
    I = 5,
    M = 6,
    SVPBMT = 7,
    SSTC = 8,
    SVINVAL = 9,
    ZIHINTPAUSE = 10,
    ZICBOM = 11,
    ZICBOZ = 12,
    ZBB = 13,
    SSAIA = 14,
    V = 15,
    SVNAPOT = 16,
    ZBA = 17,
    ZBS = 18,
    ZICNTR = 19,
    ZICSR = 20,
    ZIFENCEI = 21,
    ZIHPM = 22,
    SMSTATEEN = 23,
    ZICOND = 24,
    ZBC = 25,
    ZBKB = 26,
    ZBKC = 27,
    ZBKX = 28,
    ZKND = 29,
    ZKNE = 30,
    ZKNH = 31,
    ZKR = 32,
    ZKSED = 33,
    ZKSH = 34,
    ZKT = 35,
    ZVBB = 36,
    ZVBC = 37,
    ZVKB = 38,
    ZVKG = 39,
    ZVKNED = 40,
    ZVKNHA = 41,
    ZVKNHB = 42,
    ZVKSED = 43,
    ZVKSH = 44,
    ZVKT = 45,
    ZFH = 46,
    ZFHMIN = 47,
    ZIHINTNTL = 48,
    ZVFH = 49,
    ZVFHMIN = 50,
    ZFA = 51,
    ZTSO = 52,
    ZACAS = 53,
    SSCOFPMF = 54,
    ZIMOP = 55,
    ZCA = 56,
    ZCB = 57,
    ZCD = 58,
    ZCF = 59,
    ZCMOP = 60,
    ZAWRS = 61,
    SMNPM = 62,
    SSNPM = 63,
    SVADE = 64,
    SVADU = 65,
    SVVPTC = 66,
    ZABHA = 67,
    ZICCRSE = 68,
    ZAAMO = 69,
    ZALRSC = 70,
    ZICBOP = 71,
    ZFBFMIN = 72,
    ZVFBFMIN = 73,
    ZVFBFWMA = 74,
    ZCLSD = 75,
    ZILSD = 76,
    ZALASR = 77,
    MAX = 78,
}

impl KvmRiscvIsaExtId {
    /// Map a KVM-specific ISA extension ID to the host's `IsaExtensions`.
    ///
    /// Keep this `match` as the single source of truth; do not add a
    /// parallel array, as the two orderings will drift.
    fn to_isa_ext(self) -> Result<IsaExtensions> {
        match self {
            KvmRiscvIsaExtId::A => Ok(IsaExtensions::A),
            KvmRiscvIsaExtId::C => Ok(IsaExtensions::C),
            KvmRiscvIsaExtId::D => Ok(IsaExtensions::D),
            KvmRiscvIsaExtId::F => Ok(IsaExtensions::F),
            KvmRiscvIsaExtId::H => Ok(IsaExtensions::H),
            KvmRiscvIsaExtId::I => Ok(IsaExtensions::I),
            KvmRiscvIsaExtId::M => Ok(IsaExtensions::M),
            KvmRiscvIsaExtId::SVPBMT => Ok(IsaExtensions::SVPBMT),
            KvmRiscvIsaExtId::SSTC => Ok(IsaExtensions::SSTC),
            KvmRiscvIsaExtId::SVINVAL => Ok(IsaExtensions::SVINVAL),
            KvmRiscvIsaExtId::ZIHINTPAUSE => Ok(IsaExtensions::ZIHINTPAUSE),
            KvmRiscvIsaExtId::ZICBOM => Ok(IsaExtensions::ZICBOM),
            KvmRiscvIsaExtId::ZICBOZ => Ok(IsaExtensions::ZICBOZ),
            KvmRiscvIsaExtId::ZBB => Ok(IsaExtensions::ZBB),
            KvmRiscvIsaExtId::SSAIA => Ok(IsaExtensions::SSAIA),
            KvmRiscvIsaExtId::V => Ok(IsaExtensions::V),
            KvmRiscvIsaExtId::SVNAPOT => Ok(IsaExtensions::SVNAPOT),
            KvmRiscvIsaExtId::ZBA => Ok(IsaExtensions::ZBA),
            KvmRiscvIsaExtId::ZBS => Ok(IsaExtensions::ZBS),
            KvmRiscvIsaExtId::ZICNTR => Ok(IsaExtensions::ZICNTR),
            KvmRiscvIsaExtId::ZICSR => Ok(IsaExtensions::ZICSR),
            KvmRiscvIsaExtId::ZIFENCEI => Ok(IsaExtensions::ZIFENCEI),
            KvmRiscvIsaExtId::ZIHPM => Ok(IsaExtensions::ZIHPM),
            KvmRiscvIsaExtId::SMSTATEEN => Ok(IsaExtensions::SMSTATEEN),
            KvmRiscvIsaExtId::ZICOND => Ok(IsaExtensions::ZICOND),
            KvmRiscvIsaExtId::ZBC => Ok(IsaExtensions::ZBC),
            KvmRiscvIsaExtId::ZBKB => Ok(IsaExtensions::ZBKB),
            KvmRiscvIsaExtId::ZBKC => Ok(IsaExtensions::ZBKC),
            KvmRiscvIsaExtId::ZBKX => Ok(IsaExtensions::ZBKX),
            KvmRiscvIsaExtId::ZKND => Ok(IsaExtensions::ZKND),
            KvmRiscvIsaExtId::ZKNE => Ok(IsaExtensions::ZKNE),
            KvmRiscvIsaExtId::ZKNH => Ok(IsaExtensions::ZKNH),
            KvmRiscvIsaExtId::ZKR => Ok(IsaExtensions::ZKR),
            KvmRiscvIsaExtId::ZKSED => Ok(IsaExtensions::ZKSED),
            KvmRiscvIsaExtId::ZKSH => Ok(IsaExtensions::ZKSH),
            KvmRiscvIsaExtId::ZKT => Ok(IsaExtensions::ZKT),
            KvmRiscvIsaExtId::ZVBB => Ok(IsaExtensions::ZVBB),
            KvmRiscvIsaExtId::ZVBC => Ok(IsaExtensions::ZVBC),
            KvmRiscvIsaExtId::ZVKB => Ok(IsaExtensions::ZVKB),
            KvmRiscvIsaExtId::ZVKG => Ok(IsaExtensions::ZVKG),
            KvmRiscvIsaExtId::ZVKNED => Ok(IsaExtensions::ZVKNED),
            KvmRiscvIsaExtId::ZVKNHA => Ok(IsaExtensions::ZVKNHA),
            KvmRiscvIsaExtId::ZVKNHB => Ok(IsaExtensions::ZVKNHB),
            KvmRiscvIsaExtId::ZVKSED => Ok(IsaExtensions::ZVKSED),
            KvmRiscvIsaExtId::ZVKSH => Ok(IsaExtensions::ZVKSH),
            KvmRiscvIsaExtId::ZVKT => Ok(IsaExtensions::ZVKT),
            KvmRiscvIsaExtId::ZFH => Ok(IsaExtensions::ZFH),
            KvmRiscvIsaExtId::ZFHMIN => Ok(IsaExtensions::ZFHMIN),
            KvmRiscvIsaExtId::ZIHINTNTL => Ok(IsaExtensions::ZIHINTNTL),
            KvmRiscvIsaExtId::ZVFH => Ok(IsaExtensions::ZVFH),
            KvmRiscvIsaExtId::ZVFHMIN => Ok(IsaExtensions::ZVFHMIN),
            KvmRiscvIsaExtId::ZFA => Ok(IsaExtensions::ZFA),
            KvmRiscvIsaExtId::ZTSO => Ok(IsaExtensions::ZTSO),
            KvmRiscvIsaExtId::ZACAS => Ok(IsaExtensions::ZACAS),
            KvmRiscvIsaExtId::SSCOFPMF => Ok(IsaExtensions::SSCOFPMF),
            KvmRiscvIsaExtId::ZIMOP => Ok(IsaExtensions::ZIMOP),
            KvmRiscvIsaExtId::ZCA => Ok(IsaExtensions::ZCA),
            KvmRiscvIsaExtId::ZCB => Ok(IsaExtensions::ZCB),
            KvmRiscvIsaExtId::ZCD => Ok(IsaExtensions::ZCD),
            KvmRiscvIsaExtId::ZCF => Ok(IsaExtensions::ZCF),
            KvmRiscvIsaExtId::ZCMOP => Ok(IsaExtensions::ZCMOP),
            KvmRiscvIsaExtId::ZAWRS => Ok(IsaExtensions::ZAWRS),
            KvmRiscvIsaExtId::SMNPM => Ok(IsaExtensions::SMNPM),
            KvmRiscvIsaExtId::SSNPM => Ok(IsaExtensions::SSNPM),
            KvmRiscvIsaExtId::SVADE => Ok(IsaExtensions::SVADE),
            KvmRiscvIsaExtId::SVADU => Ok(IsaExtensions::SVADU),
            KvmRiscvIsaExtId::SVVPTC => Ok(IsaExtensions::SVVPTC),
            KvmRiscvIsaExtId::ZABHA => Ok(IsaExtensions::ZABHA),
            KvmRiscvIsaExtId::ZICCRSE => Ok(IsaExtensions::ZICCRSE),
            KvmRiscvIsaExtId::ZAAMO => Ok(IsaExtensions::ZAAMO),
            KvmRiscvIsaExtId::ZALRSC => Ok(IsaExtensions::ZALRSC),
            KvmRiscvIsaExtId::ZICBOP => Ok(IsaExtensions::ZICBOP),
            KvmRiscvIsaExtId::ZFBFMIN => Ok(IsaExtensions::ZFBFMIN),
            KvmRiscvIsaExtId::ZVFBFMIN => Ok(IsaExtensions::ZVFBFMIN),
            KvmRiscvIsaExtId::ZVFBFWMA => Ok(IsaExtensions::ZVFBFWMA),
            KvmRiscvIsaExtId::ZCLSD => Ok(IsaExtensions::ZCLSD),
            KvmRiscvIsaExtId::ZILSD => Ok(IsaExtensions::ZILSD),
            KvmRiscvIsaExtId::ZALASR => Ok(IsaExtensions::ZALASR),
            KvmRiscvIsaExtId::MAX => panic!()
        }
    }

    /// Number of real variants.
    pub const COUNT: u32 = Self::ZALASR as u32 + 1;
}

pub fn kvm_riscv_isa_check_host(kvm_ext: KvmRiscvIsaExtId) -> Result<IsaExtensions> {
    let mapped = kvm_ext.to_isa_ext()?;

    let host_ext = match mapped {
        IsaExtensions::SMNPM => IsaExtensions::SSNPM,
        other => other,
    };

    if !has_extensions(host_ext) {
        return Err(Error::new(Errno::EINVAL));
    }

    Ok(mapped)
}

pub fn kvm_riscv_isa_enable_allowed(ext: IsaExtensions) -> bool {
    match ext {
        IsaExtensions::H | IsaExtensions::SSCOFPMF |IsaExtensions::V => false,
        _ => has_extensions(ext),
    }
}
