use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_BASE: usize = 0x10;

const SBI_EXT_BASE_GET_SPEC_VERSION: usize = 0;
const SBI_EXT_BASE_GET_IMP_ID: usize = 1;
const SBI_EXT_BASE_GET_IMP_VERSION: usize = 2;
const SBI_EXT_BASE_PROBE_EXT: usize = 3;
const SBI_EXT_BASE_GET_MVENDORID: usize = 4;
const SBI_EXT_BASE_GET_MARCHID: usize = 5;
const SBI_EXT_BASE_GET_MIMPID: usize = 6;

pub(super) const VCPU_SBI_EXT_BASE: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_BASE,
    extid_end: SBI_EXT_BASE,
    default_disabled: false,
    handler: &sbi_ext_base_handler,
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

fn sbi_ext_base_handler(arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    let mut rt = KvmVcpuSbiReturn::default();

    match arch.guest_context.get_a6() {
        SBI_EXT_BASE_GET_SPEC_VERSION => {
            rt.out_val = 3 << 24;
        }
        SBI_EXT_BASE_GET_IMP_ID => {
            rt.out_val = 3;
        }
        SBI_EXT_BASE_GET_IMP_VERSION => {
            rt.out_val = 459264;
        }
        SBI_EXT_BASE_PROBE_EXT => {
            rt.out_val = 0;
        }
        SBI_EXT_BASE_GET_MVENDORID => {
            rt.out_val = arch.mvendorid;
        }
        SBI_EXT_BASE_GET_MARCHID => {
            rt.out_val = arch.marchid;
        }
        SBI_EXT_BASE_GET_MIMPID => {
            rt.out_val = arch.mimpid;
        }
        _ => {
            panic!("sbi_ext_base_handler");
        }
    }

    rt
}
