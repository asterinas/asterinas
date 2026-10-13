use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_PMU: usize = 0x504D55;

pub(super) const VCPU_SBI_EXT_PMU: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_PMU,
    extid_end: SBI_EXT_PMU,
    default_disabled: false,
    handler: &sbi_ext_pmu_handler,
    probe: Some(&sbi_ext_pmu_probe),
    init: None,
    reset: None,
    deinit: None,
    state_reg_subtype: 0,
    get_state_reg_count: None,
    get_state_reg_id: None,
    get_state_reg: None,
    set_state_reg: None,
};

fn sbi_ext_pmu_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn sbi_ext_pmu_probe() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
