use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;
use super::KVM_REG_RISCV_SBI_STA;
use super::VcpuSbiContext;

const SBI_EXT_STA: usize = 0x535441;

struct KvmRiscvSbiSta {
	shmem_lo: usize,
	shmem_hi: usize,
}

pub(super) const VCPU_SBI_EXT_STA: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_STA,
    extid_end: SBI_EXT_STA,
    default_disabled: false,
    handler: &sbi_ext_sta_handler,
    probe: Some(&sbi_ext_sta_probe),
    init: None,
    deinit: None,
    reset: Some(&kvm_sbi_ext_sta_reset),
    state_reg_subtype: KVM_REG_RISCV_SBI_STA,
    get_state_reg_count: Some(&sbi_ext_sta_get_state_reg_count),
    get_state_reg_id: None,
    get_state_reg: Some(&sbi_ext_sta_get_state_reg),
    set_state_reg: Some(&sbi_ext_sta_set_state_reg),
};

fn sbi_ext_sta_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn sbi_ext_sta_probe() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn kvm_sbi_ext_sta_reset(ctx: &mut VcpuSbiContext) -> usize {
    ctx.shmem = u64::MAX;
    ctx.last_steal = 0;
    0
}

fn sbi_ext_sta_get_state_reg_count() -> usize {
	return size_of::<KvmRiscvSbiSta>() / size_of::<usize>();
}

fn sbi_ext_sta_get_state_reg() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn sbi_ext_sta_set_state_reg() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
