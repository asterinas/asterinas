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

// pub(super) const KVM_HEDELEG_DEFAULT: usize = HEDELEG_INSTRUCTION_ADDRESS_MISALIGNED
//     | HEDELEG_ILLEGAL_INSTRUCTION
//     | HEDELEG_BREAKPOINT
//     | HEDELEG_USER_ECALL
//     | HEDELEG_HS_ECALL
//     | HEDELEG_INSTRUCTION_PAGE_FAULT
//     | HEDELEG_LOAD_PAGE_FAULT
//     | HEDELEG_STORE_AMO_PAGE_FAULT;
// pub(super) const KVM_HIDELEG_DEFAULT: usize = HIDELEG_VS_SOFTWARE_INTERRUPT
//     | HIDELEG_VS_TIMER_INTERRUPT
//     | HIDELEG_VS_EXTERNAL_INTERRUPT;
pub(super) const KVM_HEDELEG_DEFAULT: usize = 0xB30D;
pub(super) const KVM_HIDELEG_DEFAULT: usize = 0x444;


#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct KvmVcpuConfig {
	pub henvcfg: usize,
	pub hstateen0: usize,
	pub hedeleg: usize,
	pub hideleg: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VcpuRegs {
	pub zero: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
}

#[repr(C)]
pub(crate) struct VcpuCsrs {
	// 在进入 guest 前需要设置的
	pub(crate) sepc: usize,
	pub(crate) sstatus: usize,
	// 专用于限制 V = 1 特权级的
	pub(crate) hstatus: usize,
	pub(crate) hedeleg: usize,
	pub(crate) hideleg: usize,
	pub(crate) hcounteren: usize,
	pub(crate) henvcfg: usize,
	pub(crate) hie: usize,
	pub(crate) hip: usize,
	pub(crate) hvip: usize,
	// 没有对应的 vCSR，要切换给 guest
	pub(crate) scounteren: usize,
	pub(crate) senvcfg: usize,
	// 要设置的 guest vCSR
	// 不用给 host 保存，因为各用一套
	// 每次进入 guest 重新设置，以防切换 pCPU
	// 每次退出 guest 保存，以防切换 pCPU，（可能）用于帮助模拟
	pub(crate) vsatp: usize,
	pub(crate) vsie: usize,
	pub(crate) vscause: usize,
	pub(crate) vsepc: usize,
	pub(crate) vsscratch: usize,
	pub(crate) vsstatus: usize,
	pub(crate) vstval: usize,
	pub(crate) vstvec: usize,
}

// 如果初始化为全零，不如派生 Default
impl VcpuRegs {
	pub fn new() -> Self {
		Self {
			zero: 0,
			ra: 0,
			sp: 0,
			gp: 0,
			tp: 0,
			t0: 0,
			t1: 0,
			t2: 0,
			s0: 0,
			s1: 0,
			a0: 0,
			a1: 0,
			a2: 0,
			a3: 0,
			a4: 0,
			a5: 0,
			a6: 0,
			a7: 0,
			s2: 0,
			s3: 0,
			s4: 0,
			s5: 0,
			s6: 0,
			s7: 0,
			s8: 0,
			s9: 0,
			s10: 0,
			s11: 0,
			t3: 0,
			t4: 0,
			t5: 0,
			t6: 0,
		}
	}
}

impl VcpuCsrs {
	pub fn new() -> Self {
		const SSTATUS_SPP: usize = 1 << 8;
		// const SSTATUS_SPIE: usize = 1 << 5;

		const HSTATUS_VTW: usize = 1 << 21;
		const HSTATUS_SPVP: usize = 1 << 8;
		const HSTATUS_SPV: usize = 1 << 7;

		const HEDELEG_INSTRUCTION_ADDRESS_MISALIGNED: usize = 1 << 0;
		const HEDELEG_ILLEGAL_INSTRUCTION: usize = 1 << 2;
		const HEDELEG_BREAKPOINT: usize = 1 << 3;
		const HEDELEG_USER_ECALL: usize = 1 << 8;
		const HEDELEG_INSTRUCTION_PAGE_FAULT: usize = 1 << 12;
		const HEDELEG_LOAD_PAGE_FAULT: usize = 1 << 13;
		const HEDELEG_STORE_AMO_PAGE_FAULT: usize = 1 << 15;

		const HIDELEG_VS_EXTERNAL_INTERRUPT: usize = 1 << 10;
		const HIDELEG_VS_TIMER_INTERRUPT: usize = 1 << 6;
		const HIDELEG_VS_SOFTWARE_INTERRUPT: usize = 1 << 2;

		// const HENVCFG_STCE:usize = 0x1 << 63;
		// const HENVCFG_ADUE:usize = 0x1 << 61;
		// const HENVCFG_CBIE:usize = 0x3 << 4;
		// const HENVCFG_CBCFE:usize = 0x1 << 6;
		// const HENVCFG_CBZE:usize = 0x1 << 7;
		
		let hedeleg = HEDELEG_INSTRUCTION_ADDRESS_MISALIGNED |
			HEDELEG_ILLEGAL_INSTRUCTION |
			HEDELEG_BREAKPOINT |
			HEDELEG_USER_ECALL |
			HEDELEG_INSTRUCTION_PAGE_FAULT |
			HEDELEG_LOAD_PAGE_FAULT |
			HEDELEG_STORE_AMO_PAGE_FAULT;

		let hideleg = HIDELEG_VS_EXTERNAL_INTERRUPT |
			HIDELEG_VS_TIMER_INTERRUPT |
			HIDELEG_VS_SOFTWARE_INTERRUPT;

		let henvcfg = 0;

		let hstatus = HSTATUS_VTW | HSTATUS_SPVP | HSTATUS_SPV;

		Self {
			// 在进入 guest 前需要设置的
			sepc: 0,
			sstatus: SSTATUS_SPP,
			// 专用于限制 V = 1 特权级的
			hcounteren: 0,
			hedeleg,
			hideleg,
			henvcfg,
			hstatus,
			hvip: 0,
			hie: 0,
			hip: 0,

			// 没有对应的 vCSR，要切换给 guest
			scounteren: 0,
			senvcfg: 0,
			// 要设置的 guest vCSR
			// 不用给 host 保存，因为各用一套
			// 每次进入 guest 重新设置，以防切换 pCPU
			// 每次退出 guest 保存，以防切换 pCPU，（可能）用于帮助模拟
			vsatp: 0,
			vsie: 0,
			vscause: 0,
			vsepc: 0,
			vsscratch: 0,
			vsstatus: 0,
			vstval: 0,
			vstvec: 0,
		}
	}
}
