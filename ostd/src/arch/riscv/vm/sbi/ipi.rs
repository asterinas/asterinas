use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_IPI: usize = 0x735049;

pub(super) const VCPU_SBI_EXT_IPI: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_IPI,
    extid_end: SBI_EXT_IPI,
    default_disabled: false,
    handler: &sbi_ext_ipi_handler,
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

fn sbi_ext_ipi_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
