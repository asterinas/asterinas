use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_SUSP: usize = 0x53555350;

pub(super) const VCPU_SBI_EXT_SUSP: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_SUSP,
    extid_end: SBI_EXT_SUSP,
    default_disabled: true,
    handler: &sbi_ext_susp_handler,
    probe: None,
    init: None,
    deinit: None,
    reset: None,
    state_reg_subtype: 0,
    get_state_reg_count: None,
    get_state_reg_id: None,
    get_state_reg: None,
    set_state_reg: None,
};

fn sbi_ext_susp_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
