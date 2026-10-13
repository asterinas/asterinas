use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_HSM: usize = 0x48534D;

pub(super) const VCPU_SBI_EXT_HSM: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_HSM,
    extid_end: SBI_EXT_HSM,
    default_disabled: false,
    handler: &sbi_ext_hsm_handler,
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

fn sbi_ext_hsm_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
