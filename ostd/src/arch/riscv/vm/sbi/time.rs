use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_TIME: usize = 0x54494D45;

pub(super) const VCPU_SBI_EXT_TIME: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_TIME,
    extid_end: SBI_EXT_TIME,
    default_disabled: false,
    handler: &sbi_ext_time_handler,
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

fn sbi_ext_time_handler(arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    let next_cycle = arch.guest_context.get_a0();
    arch.timer.call_next_event(next_cycle);
    KvmVcpuSbiReturn::default()
}
