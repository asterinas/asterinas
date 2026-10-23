// SPDX-License-Identifier: MPL-2.0

use crate::{
    Error,
    arch::cpu::extension::{IsaExtensions, has_extensions},
    prelude::*,
};

pub(super) const KVM_RISCV_BASE_ISA_MASK: usize = 0x0000000003FFFFFF;

/// ISA extension IDs specific to KVM.
/// This is not the same as the host ISA extension IDs,
/// as that is internal to the host and should not be exposed to the guest.
/// This should always be contiguous
/// to keep the mapping simple in the KVM implementation.
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
    Svpbmt,
    Sstc,
    Svinval,
    Zihintpause,
    Zicbom,
    Zicboz,
    Zbb,
    Ssaia,
    V,
    Svnapot,
    Zba,
    Zbs,
    Zicntr,
    Zicsr,
    Zifencei,
    Zihpm,
    Smstateen,
    Zicond,
    Zbc,
    Zbkb,
    Zbkc,
    Zbkx,
    Zknd,
    Zkne,
    Zknh,
    Zkr,
    Zksed,
    Zksh,
    Zkt,
    Zvbb,
    Zvbc,
    Zvkb,
    Zvkg,
    Zvkned,
    Zvknha,
    Zvknhb,
    Zvksed,
    Zvksh,
    Zvkt,
    Zfh,
    Zfhmin,
    Zihintntl,
    Zvfh,
    Zvfhmin,
    Zfa,
    Ztso,
    Zacas,
    Sscofpmf,
    Zimop,
    Zca,
    Zcb,
    Zcd,
    Zcf,
    Zcmop,
    Zawrs,
    Smnpm,
    Ssnpm,
    Svade,
    Svadu,
    Svvptc,
    Zabha,
    Ziccrse,
    Zaamo,
    Zalrsc,
    Zicbop,
    Zfbfmin,
    Zvfbfmin,
    Zvfbfwma,
    Zclsd,
    Zilsd,
    Zalasr,
}

impl KvmRiscvIsaExtId {
    /// Map a KVM-specific ISA extension ID to the host's `IsaExtensions`.
    ///
    /// Keep this `match` as the single source of truth; do not add a
    /// parallel array, as the two orderings will drift.
    fn to_isa_ext(self) -> Option<IsaExtensions> {
        match self {
            KvmRiscvIsaExtId::A => Some(IsaExtensions::A),
            KvmRiscvIsaExtId::C => Some(IsaExtensions::C),
            KvmRiscvIsaExtId::D => Some(IsaExtensions::D),
            KvmRiscvIsaExtId::F => Some(IsaExtensions::F),
            KvmRiscvIsaExtId::H => Some(IsaExtensions::H),
            KvmRiscvIsaExtId::I => Some(IsaExtensions::I),
            KvmRiscvIsaExtId::M => Some(IsaExtensions::M),
            KvmRiscvIsaExtId::Svpbmt => Some(IsaExtensions::SVPBMT),
            KvmRiscvIsaExtId::Sstc => Some(IsaExtensions::SSTC),
            KvmRiscvIsaExtId::Svinval => Some(IsaExtensions::SVINVAL),
            KvmRiscvIsaExtId::Zihintpause => Some(IsaExtensions::ZIHINTPAUSE),
            KvmRiscvIsaExtId::Zicbom => Some(IsaExtensions::ZICBOM),
            KvmRiscvIsaExtId::Zicboz => Some(IsaExtensions::ZICBOZ),
            KvmRiscvIsaExtId::Zbb => Some(IsaExtensions::ZBB),
            KvmRiscvIsaExtId::Ssaia => Some(IsaExtensions::SSAIA),
            KvmRiscvIsaExtId::V => Some(IsaExtensions::V),
            KvmRiscvIsaExtId::Svnapot => Some(IsaExtensions::SVNAPOT),
            KvmRiscvIsaExtId::Zba => Some(IsaExtensions::ZBA),
            KvmRiscvIsaExtId::Zbs => Some(IsaExtensions::ZBS),
            KvmRiscvIsaExtId::Zicntr => Some(IsaExtensions::ZICNTR),
            KvmRiscvIsaExtId::Zicsr => Some(IsaExtensions::ZICSR),
            KvmRiscvIsaExtId::Zifencei => Some(IsaExtensions::ZIFENCEI),
            KvmRiscvIsaExtId::Zihpm => Some(IsaExtensions::ZIHPM),
            KvmRiscvIsaExtId::Smstateen => Some(IsaExtensions::SMSTATEEN),
            KvmRiscvIsaExtId::Zicond => Some(IsaExtensions::ZICOND),
            KvmRiscvIsaExtId::Zbc => Some(IsaExtensions::ZBC),
            KvmRiscvIsaExtId::Zbkb => Some(IsaExtensions::ZBKB),
            KvmRiscvIsaExtId::Zbkc => Some(IsaExtensions::ZBKC),
            KvmRiscvIsaExtId::Zbkx => Some(IsaExtensions::ZBKX),
            KvmRiscvIsaExtId::Zknd => Some(IsaExtensions::ZKND),
            KvmRiscvIsaExtId::Zkne => Some(IsaExtensions::ZKNE),
            KvmRiscvIsaExtId::Zknh => Some(IsaExtensions::ZKNH),
            KvmRiscvIsaExtId::Zkr => Some(IsaExtensions::ZKR),
            KvmRiscvIsaExtId::Zksed => Some(IsaExtensions::ZKSED),
            KvmRiscvIsaExtId::Zksh => Some(IsaExtensions::ZKSH),
            KvmRiscvIsaExtId::Zkt => Some(IsaExtensions::ZKT),
            KvmRiscvIsaExtId::Zvbb => Some(IsaExtensions::ZVBB),
            KvmRiscvIsaExtId::Zvbc => Some(IsaExtensions::ZVBC),
            KvmRiscvIsaExtId::Zvkb => Some(IsaExtensions::ZVKB),
            KvmRiscvIsaExtId::Zvkg => Some(IsaExtensions::ZVKG),
            KvmRiscvIsaExtId::Zvkned => Some(IsaExtensions::ZVKNED),
            KvmRiscvIsaExtId::Zvknha => Some(IsaExtensions::ZVKNHA),
            KvmRiscvIsaExtId::Zvknhb => Some(IsaExtensions::ZVKNHB),
            KvmRiscvIsaExtId::Zvksed => Some(IsaExtensions::ZVKSED),
            KvmRiscvIsaExtId::Zvksh => Some(IsaExtensions::ZVKSH),
            KvmRiscvIsaExtId::Zvkt => Some(IsaExtensions::ZVKT),
            KvmRiscvIsaExtId::Zfh => Some(IsaExtensions::ZFH),
            KvmRiscvIsaExtId::Zfhmin => Some(IsaExtensions::ZFHMIN),
            KvmRiscvIsaExtId::Zihintntl => Some(IsaExtensions::ZIHINTNTL),
            KvmRiscvIsaExtId::Zvfh => Some(IsaExtensions::ZVFH),
            KvmRiscvIsaExtId::Zvfhmin => Some(IsaExtensions::ZVFHMIN),
            KvmRiscvIsaExtId::Zfa => Some(IsaExtensions::ZFA),
            KvmRiscvIsaExtId::Ztso => Some(IsaExtensions::ZTSO),
            KvmRiscvIsaExtId::Zacas => Some(IsaExtensions::ZACAS),
            KvmRiscvIsaExtId::Sscofpmf => Some(IsaExtensions::SSCOFPMF),
            KvmRiscvIsaExtId::Zimop => Some(IsaExtensions::ZIMOP),
            KvmRiscvIsaExtId::Zca => Some(IsaExtensions::ZCA),
            KvmRiscvIsaExtId::Zcb => Some(IsaExtensions::ZCB),
            KvmRiscvIsaExtId::Zcd => Some(IsaExtensions::ZCD),
            KvmRiscvIsaExtId::Zcf => Some(IsaExtensions::ZCF),
            KvmRiscvIsaExtId::Zcmop => Some(IsaExtensions::ZCMOP),
            KvmRiscvIsaExtId::Zawrs => Some(IsaExtensions::ZAWRS),
            KvmRiscvIsaExtId::Smnpm => Some(IsaExtensions::SMNPM),
            KvmRiscvIsaExtId::Ssnpm => Some(IsaExtensions::SSNPM),
            KvmRiscvIsaExtId::Svade => Some(IsaExtensions::SVADE),
            KvmRiscvIsaExtId::Svadu => Some(IsaExtensions::SVADU),
            KvmRiscvIsaExtId::Svvptc => Some(IsaExtensions::SVVPTC),
            KvmRiscvIsaExtId::Zabha => Some(IsaExtensions::ZABHA),
            KvmRiscvIsaExtId::Ziccrse => Some(IsaExtensions::ZICCRSE),
            KvmRiscvIsaExtId::Zaamo => Some(IsaExtensions::ZAAMO),
            KvmRiscvIsaExtId::Zalrsc => Some(IsaExtensions::ZALRSC),
            KvmRiscvIsaExtId::Zicbop => Some(IsaExtensions::ZICBOP),
            KvmRiscvIsaExtId::Zfbfmin => Some(IsaExtensions::ZFBFMIN),
            KvmRiscvIsaExtId::Zvfbfmin => Some(IsaExtensions::ZVFBFMIN),
            KvmRiscvIsaExtId::Zvfbfwma => Some(IsaExtensions::ZVFBFWMA),
            KvmRiscvIsaExtId::Zclsd => Some(IsaExtensions::ZCLSD),
            KvmRiscvIsaExtId::Zilsd => Some(IsaExtensions::ZILSD),
            KvmRiscvIsaExtId::Zalasr => Some(IsaExtensions::ZALASR),
        }
    }

    /// Convert a raw KVM ISA extension ID into the enum variant.
    ///
    /// Returns `None` for any out-of-range value.
    fn from_raw(raw: u32) -> Option<Self> {
        if raw > Self::Zalasr as u32 {
            return None;
        }
        // SAFETY: `Self` is `#[repr(u32)]` and contiguous from 0 to Zalasr.
        Some(unsafe { core::mem::transmute::<u32, Self>(raw) })
    }

    /// Number of real variants.
    const COUNT: u32 = Self::Zalasr as u32 + 1;
}

fn kvm_riscv_isa_check_host(kvm_ext: KvmRiscvIsaExtId) -> Option<IsaExtensions> {
    if let Some(mapped) = kvm_ext.to_isa_ext() {
        let host_ext = match mapped {
            IsaExtensions::SMNPM => IsaExtensions::SSNPM,
            other => other,
        };

        if !has_extensions(host_ext) {
            return None;
        }

        Some(mapped)
    } else {
        None
    }
}

fn kvm_riscv_isa_enable_allowed(ext: IsaExtensions) -> bool {
    match ext {
        IsaExtensions::H | IsaExtensions::SSCOFPMF | IsaExtensions::V => false,
        _ => has_extensions(ext),
    }
}

pub(super) fn kvm_riscv_vcpu_setup_isa(isa: &mut u128) -> Result<()> {
    for raw in 0..KvmRiscvIsaExtId::COUNT {
        let ext = KvmRiscvIsaExtId::from_raw(raw).ok_or(Error::InvalidArgs)?;

        if let Some(isa_ext) = kvm_riscv_isa_check_host(ext)
            && kvm_riscv_isa_enable_allowed(isa_ext)
        {
            *isa |= 0x1 << (isa_ext.bits().trailing_zeros() as usize);
        }
    }
    Ok(())
}
