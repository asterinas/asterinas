/// 1

use crate::prelude::*;
use crate::arch::vm::vcpu::VcpuArchInner;
use super::vcpu::KvmCpuTrap;
use super::onereg::KVM_REG_RISCV_SUBTYPE_SHIFT;
use super::onereg::{KVM_REG_RISCV, KVM_REG_SIZE_U64, KVM_REG_RISCV_SBI_STATE};

mod base;
mod forward;
mod fwft;
mod hsm;
mod ipi;
mod pmu;
mod rfence;
mod srst;
mod sta;
mod susp;
mod time;
mod v01;

use base::VCPU_SBI_EXT_BASE;
use forward::{VCPU_SBI_EXT_DBCN, VCPU_SBI_EXT_EXPERIMENTAL, VCPU_SBI_EXT_MPXY, VCPU_SBI_EXT_VENDOR};
use fwft::{KvmSbiFwft, VCPU_SBI_EXT_FWFT};
use hsm::VCPU_SBI_EXT_HSM;
use ipi::VCPU_SBI_EXT_IPI;
use pmu::VCPU_SBI_EXT_PMU;
use rfence::VCPU_SBI_EXT_RFENCE;
use srst::VCPU_SBI_EXT_SRST;
use sta::VCPU_SBI_EXT_STA;
use susp::VCPU_SBI_EXT_SUSP;
use time::VCPU_SBI_EXT_TIME;
use v01::VCPU_SBI_EXT_V01;

const KVM_REG_RISCV_SBI_STA: usize = 0x0 << KVM_REG_RISCV_SUBTYPE_SHIFT;
const KVM_REG_RISCV_SBI_FWFT: usize = 0x1 << KVM_REG_RISCV_SUBTYPE_SHIFT;

/// The status of SBI extensions
#[derive(Clone, Copy, PartialEq)]
pub(super) enum KvmRiscvSbiExtStatus {
	KvmRiscvSbiExtStatusUninitialized,
	KvmRiscvSbiExtStatusUnavailable,
	KvmRiscvSbiExtStatusEnabled,
	KvmRiscvSbiExtStatusDisabled,
}

/// SBI extension IDs specific to KVM. This is not the same as the SBI
/// extension IDs defined by the RISC-V SBI specification.
#[derive(Clone, Copy)]
pub(super) enum KvmRiscvSbiExtId {
	KvmRiscvSbiExtV01,
	KvmRiscvSbiExtTime,
	KvmRiscvSbiExtIpi,
	KvmRiscvSbiExtRfence,
	KvmRiscvSbiExtSrst,
	KvmRiscvSbiExtHsm,
	KvmRiscvSbiExtPmu,
	KvmRiscvSbiExtExperimental,
	KvmRiscvSbiExtVendor,
	KvmRiscvSbiExtDbcn,
	KvmRiscvSbiExtSta,
	KvmRiscvSbiExtSusp,
	KvmRiscvSbiExtFwft,
	KvmRiscvSbiExtMpxy,
	KvmRiscvSbiExtMax,
}

/// 1
#[repr(C)]
pub(super) struct VcpuSbiContext {
	pub(super) return_handled: usize,
	pub(super) ext_status: [KvmRiscvSbiExtStatus; KvmRiscvSbiExtId::KvmRiscvSbiExtMax as usize + 1],
    /// Firmware feature SBI extension context
	fwft_context: KvmSbiFwft,
    pub(super) shmem: u64,
	pub(super) last_steal:u64,
}

impl VcpuSbiContext {
    /// 1
    pub(super) fn new() -> Self {
        Self {
            return_handled: 0,
            ext_status: [KvmRiscvSbiExtStatus::KvmRiscvSbiExtStatusUninitialized; KvmRiscvSbiExtId::KvmRiscvSbiExtMax as usize + 1],
            fwft_context: KvmSbiFwft::new(),
            shmem: 0,
            last_steal: 0,
        }
    }

    /// 1
    pub(super) fn init(&mut self) {
        for i in 0..15 {
            let entry = &SBI_EXT_TABLE[i];
            let idx = entry.ext_idx as usize;

            if entry.ext.default_disabled {
                self.ext_status[idx] = KvmRiscvSbiExtStatus::KvmRiscvSbiExtStatusDisabled;
            } else {
                self.ext_status[idx] = KvmRiscvSbiExtStatus::KvmRiscvSbiExtStatusEnabled;
            }

            if let Some(func) = entry.ext.init {
                func(self);
            }
        }
    }
}

/// 1
struct KvmRiscvSbiExtensionEntry {
	ext_idx: KvmRiscvSbiExtId,
	ext: &'static KvmVcpuSbiExtension,
}

/// 1
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct KvmVcpuSbiReturn {
	pub out_val: usize,
	pub err_val: usize,
	pub utrap: KvmCpuTrap,
	pub uexit: bool,
}

pub(super) struct KvmCpuSbiReturn {
	out_val: usize,
	err_val: usize,
	utrap: KvmCpuTrap,
	uexit: bool,
}

/// A type alias for the Sbi Handler callback function.
pub(super) type SbiHandlerFunction = dyn Fn(&VcpuArchInner) -> KvmVcpuSbiReturn + Sync + Send + 'static;
pub(super) type SbiProbeFunction = dyn Fn() -> KvmVcpuSbiReturn + Sync + Send + 'static;
pub(super) type SbiInitFunction = dyn Fn(&mut VcpuSbiContext) -> usize + Sync + Send + 'static;
pub(super) type SbiDeinitFunction = dyn Fn() -> KvmVcpuSbiReturn + Sync + Send + 'static;
pub(super) type SbiResetFunction = dyn Fn(&mut VcpuSbiContext) -> usize + Sync + Send + 'static;
pub(super) type SbiGetStateRegIdFunction = dyn Fn(usize) -> usize + Sync + Send + 'static;
pub(super) type SbiGetStateRegCountFunction = dyn Fn() -> usize + Sync + Send + 'static;
pub(super) type SbiGetStateRegFunction = dyn Fn() -> KvmVcpuSbiReturn + Sync + Send + 'static;
pub(super) type SbiSetStateRegFunction = dyn Fn() -> KvmVcpuSbiReturn + Sync + Send + 'static;

/// 1
struct KvmVcpuSbiExtension {
	extid_start: usize,
	extid_end: usize,

	default_disabled: bool,

	/// SBI extension handler. It can be defined for a given extension or group of
	/// extension. But it should always return linux error codes rather than SBI
	/// specific error codes.
	handler: &'static SbiHandlerFunction,

    /// Extension specific probe function
    probe: Option<&'static SbiProbeFunction>,

	/// Init/deinit function called once during VCPU init/destroy. These
	/// might be use if the SBI extensions need to allocate or do specific
	/// init time only configuration.
	init: Option<&'static SbiInitFunction>,
    deinit: Option<&'static SbiDeinitFunction>,

    reset: Option<&'static SbiResetFunction>,

    state_reg_subtype: usize,
    get_state_reg_count: Option<&'static SbiGetStateRegCountFunction>,
	get_state_reg_id: Option<&'static SbiGetStateRegIdFunction>,
    get_state_reg: Option<&'static SbiGetStateRegFunction>,
    set_state_reg: Option<&'static SbiSetStateRegFunction>,
}

/// 1
const SBI_EXT_TABLE: [KvmRiscvSbiExtensionEntry; 15] = [
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtV01,
        ext: &VCPU_SBI_EXT_V01,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtMax,
        ext: &VCPU_SBI_EXT_BASE,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtTime,
        ext: &VCPU_SBI_EXT_TIME,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtIpi,
        ext: &VCPU_SBI_EXT_IPI,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtRfence,
        ext: &VCPU_SBI_EXT_RFENCE,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtSrst,
        ext: &VCPU_SBI_EXT_SRST,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtHsm,
        ext: &VCPU_SBI_EXT_HSM,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtPmu,
        ext: &VCPU_SBI_EXT_PMU,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtDbcn,
        ext: &VCPU_SBI_EXT_DBCN,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtSusp,
        ext: &VCPU_SBI_EXT_SUSP,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtSta,
        ext: &VCPU_SBI_EXT_STA,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtFwft,
        ext: &VCPU_SBI_EXT_FWFT,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtMpxy,
        ext: &VCPU_SBI_EXT_MPXY,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtExperimental,
        ext: &VCPU_SBI_EXT_EXPERIMENTAL,
    },
    KvmRiscvSbiExtensionEntry {
        ext_idx: KvmRiscvSbiExtId::KvmRiscvSbiExtVendor,
        ext: &VCPU_SBI_EXT_VENDOR,
    },
];

fn vcpu_get_sbi_ext_idx(idx: usize) -> usize {
	for i in 0..15 {
        if SBI_EXT_TABLE[i].ext_idx as usize == idx {
            return i;
        }
    }
	100
}

pub(super) fn reset_sbi(inner: &mut VcpuArchInner) {
    for i in 0..15 {
        let entry = &SBI_EXT_TABLE[i];
        if let Some(func) = entry.ext.reset {
            if inner.sbi_context.ext_status[entry.ext_idx as usize] != KvmRiscvSbiExtStatus::KvmRiscvSbiExtStatusEnabled {
                func(&mut inner.sbi_context);
            }
        }
    }
}

pub(super) fn handle_sbi_call(inner: &mut VcpuArchInner) {
    let index = vcpu_get_sbi_ext_idx(inner.guest_context.get_a7());
    if index !=100 {
        let ext = &SBI_EXT_TABLE[index];
        let func = ext.ext.handler;
        
        let rt = func(inner);

        if !rt.uexit {
            inner.guest_context.set_a0(rt.err_val);
        }

        inner.guest_context.step_sepc(4);
        inner.guest_context.set_a1(rt.out_val);
    } else {
        inner.guest_context.set_a0(usize::MAX - 1);
        inner.guest_context.step_sepc(4);
        inner.guest_context.set_a1(0);
    }
}

pub(super) fn state_reg_count(inner: &VcpuArchInner, mut vec: Option<&mut Vec<usize>>) -> usize {
    let mut n = 0;

        for i in 0..15 {
            let entry = &SBI_EXT_TABLE[i];

            if entry.ext.get_state_reg_count.is_none() 
            || inner.sbi_context.ext_status[entry.ext_idx as usize] != KvmRiscvSbiExtStatus::KvmRiscvSbiExtStatusEnabled {
                continue;
            }

            if let Some(func) = entry.ext.get_state_reg_count {
                let state_reg_count = func();

                for j in 0..state_reg_count {
                    if let Some(vec) = vec.as_deref_mut() {
                        if let Some(func) = entry.ext.get_state_reg_id {
                            vec.push(func(j));
                        } else {
                            let reg = KVM_REG_RISCV
                            | KVM_REG_SIZE_U64
                            | KVM_REG_RISCV_SBI_STATE 
                            | entry.ext.state_reg_subtype
                            | j;
                            vec.push(reg);
                        }
                    }
                }
                n+=state_reg_count;
            }
        }

    n
}
