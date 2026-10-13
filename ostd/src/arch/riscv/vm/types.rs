pub(super) const SATP_MODE_48:u64 = 0x9000000000000000;
pub(super) const SATP_MODE_SHIFT:u64 = 	60;

const IRQ_S_SOFT:u64 =		1;
const IRQ_VS_SOFT:u64 =		2;
const IRQ_S_TIMER:u64 =		5;
const IRQ_S_EXT:u64 =		9;
const IRQ_PMU_OVF:u64 =		13;
pub(super) const IRQ_LOCAL_MASK:u64 = 0x3FFF;

pub(super) const VSIP_TO_HVIP_SHIFT: u64 = 	IRQ_VS_SOFT - IRQ_S_SOFT;
pub(super) const VSIP_VALID_MASK:u64 =	(1 << IRQ_S_SOFT) | (1 << IRQ_S_TIMER) | (1 << IRQ_S_EXT) | (1 << IRQ_PMU_OVF);

pub(super) const KVM_HEDELEG_DEFAULT:usize = 0xB30D;
pub(super) const KVM_HIDELEG_DEFAULT:usize = 0x444;

#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct KvmVcpuConfig {
	pub henvcfg: usize,
	pub hstateen0: usize,
	pub hedeleg: usize,
	pub hideleg: usize,
}
