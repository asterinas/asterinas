use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;

const SBI_EXT_DBCN: usize = 0x4442434E;
const SBI_EXT_NACL: usize = 0x4E41434C;
const SBI_EXT_MPXY: usize = 0x4D505859;
const SBI_EXT_DBTR: usize = 0x44425452;
const SBI_EXT_EXPERIMENTAL_START: usize = 0x08000000;
const SBI_EXT_EXPERIMENTAL_END: usize = 0x08FFFFFF;
const SBI_EXT_VENDOR_START: usize = 0x09000000;
const SBI_EXT_VENDOR_END: usize = 0x09FFFFFF;

pub(super) const VCPU_SBI_EXT_DBCN: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_DBCN,
    extid_end: SBI_EXT_DBCN,
    default_disabled: true,
    handler: &sbi_forward_handler,
    probe:None,
    init: None,
    deinit: None,
    reset: None,
    state_reg_subtype: 0,
    get_state_reg_count: None,
    get_state_reg_id: None,
    get_state_reg: None,
    set_state_reg: None,
};

pub(super) const VCPU_SBI_EXT_MPXY: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_MPXY,
    extid_end: SBI_EXT_MPXY,
    default_disabled: true,
    handler: &sbi_forward_handler,
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

pub(super) const VCPU_SBI_EXT_EXPERIMENTAL: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_EXPERIMENTAL_START,
    extid_end: SBI_EXT_EXPERIMENTAL_END,
    default_disabled: false,
    handler: &sbi_forward_handler,
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

pub(super) const VCPU_SBI_EXT_VENDOR: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_VENDOR_START,
    extid_end: SBI_EXT_VENDOR_END,
    default_disabled: false,
    handler: &sbi_forward_handler,
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

fn sbi_forward_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
