
use crate::prelude::*;

///1

use super::onereg::*;
use crate::arch::{cpu::extension::{IsaExtensions, has_extensions}, vm::context::VcpuContext};

use crate::arch::vm::types::*;
use crate::arch::vm::timer::*;
use crate::arch::vm::sbi::*;
use crate::arch::vm::isa::*;
use crate::arch::vm::csr::*;

/// 1
#[derive(Clone, Copy, Debug, Default)]
pub struct KvmCpuTrap {
    /// 1
	pub sepc: usize,
    /// 1
	pub scause: usize,
    /// 1
	pub stval: usize,
    /// 1
	pub htval: usize,
    /// 1
	pub htinst: usize,
}

/// 1
#[repr(C)]
pub struct VcpuArchInner {
    /// VCPU ran at least once
    ran_atleast_once: bool,
    /// Last Host CPU on which Guest VCPU exited
    last_exit_cpu: i32,
    /// ISA feature bits (similar to MISA)
    pub(super) isa: [u64; 2],
    /// Vendor details
	pub(super) mvendorid: usize,
    /// Arch details
	pub(super) marchid: usize,
    /// Implementation details
	pub(super) mimpid: usize,

	/// SSCRATCH, STVEC, and SCOUNTEREN of Host
	pub(super) host_sscratch: usize,
	pub(super) host_stvec: usize,
    /// Host scounteren
	host_scounteren: usize,
    /// Host senvcfg
	host_senvcfg: usize,
	host_sstateen0: usize,

    /// CPU context of Host
	pub host_context: VcpuContext,

	/// CPU context of Guest VCPU
	pub guest_context: VcpuContext,

    /// CPU CSR context of Guest VCPU
	pub(super) guest_csr: KvmVcpuCsr,

	/// CPU Smstateen CSR context of Guest VCPU, TODO
	// struct kvm_vcpu_smstateen_csr smstateen_csr;

    /*
	 * VCPU interrupts
	 *
	 * The irqs_pending bitmap represents pending interrupts whereas
	 * irqs_pending_mask represents bits changed in irqs_pending. Updates
	 * to these bitmaps are serialized so vcpu interrupt sync/flush cannot
	 * drop a newly injected interrupt while syncing guest-visible HVIP.
	 */
    irqs_pending: u64,
    /// 1
    irqs_pending_mask: u64,

    /// VCPU Timer
	pub(super) timer: VcpuTimer,

    /// SBI context
	pub(super) sbi_context: VcpuSbiContext,

    /// 'static' configurations which are set only once
	cfg: KvmVcpuConfig,
}

impl VcpuArchInner {
    /// 1
    pub fn new() -> Self {
        Self {
            ran_atleast_once: false,
            last_exit_cpu: 0,
            isa: [0,0],
            mvendorid: 0,
            marchid: 0,
            mimpid: 0,
            host_sscratch: 0,
            host_stvec: 0,
            host_scounteren: 0,
            host_senvcfg: 0,
            host_sstateen0: 0,
            host_context: VcpuContext::default(),
            guest_context: VcpuContext::default(),
            guest_csr: KvmVcpuCsr::default(),
            irqs_pending: 0,
            irqs_pending_mask: 0,
            timer: VcpuTimer::default(),
            sbi_context: VcpuSbiContext::new(),
            cfg: KvmVcpuConfig::default(),
        }
    }

    /// 1
    pub fn init(&mut self) {
        // Setup VCPU config
        self.cfg.hedeleg = KVM_HEDELEG_DEFAULT;
        self.cfg.hideleg = KVM_HIDELEG_DEFAULT;

        // Setup ISA features available to VCPU
        kvm_riscv_vcpu_setup_isa(&mut self.isa);

        // Setup vendor, arch, and implementation details
	    self.mvendorid = sbi_rt::get_mvendorid();
	    self.marchid = sbi_rt::get_marchid();
	    self.mimpid = sbi_rt::get_mimpid();

        // Setup VCPU timer
        self.timer.init();

        // Setup SBI extensions
	    // NOTE: This must be the last thing to be initialized.
        self.sbi_context.init();

        // Reset VCPU
        self.reset(false);
    }

    /// 1
    pub fn is_ran_at_least_once(&self) -> bool {
        self.ran_atleast_once
    }

    /// 1
    pub fn ran_at_least_once(&mut self) {
        self.ran_atleast_once = true;
    }

    /// 1
    pub fn reset(&mut self, _kvm_sbi_reset: bool) {
        self.last_exit_cpu = -1;

        self.context_reset();

        self.fp_reset();

        self.timer_reset();

        self.sbi_reset();
    }

    /// 1
    pub fn context_reset(&mut self) {
        self.guest_context = VcpuContext::default();
        self.guest_csr = KvmVcpuCsr::default();

        // Setup reset state of shadow SSTATUS and HSTATUS CSRs
        self.guest_context.set_sstatus(SR_SPP | SR_SPIE);

        let mut hstatus = self.guest_context.get_hstatus();
        hstatus |= HSTATUS_VTW;
        hstatus |= HSTATUS_SPVP;
        hstatus |= HSTATUS_SPV;
        self.guest_context.set_hstatus(hstatus);
    }

    /// 1
    pub fn fp_reset(&mut self) {
        let mut sstatus = self.guest_context.get_sstatus();
        sstatus &= !(0x6000);

        if has_extensions(IsaExtensions::F) || has_extensions(IsaExtensions::D) {
            sstatus |= 0x2000;
        }
        self.guest_context.set_sstatus(sstatus);
    }

    /// 1
    pub fn timer_reset(&mut self) {
        self.timer.reset();
    }

    /// 1
    pub fn sbi_reset(&mut self) {
        reset_sbi(self)
    }

    ///1
    pub fn config_ran_once(&mut self) {
        // SSTC
        self.cfg.henvcfg |= ENVCFG_STCE;
        // ZICBOM
        self.cfg.henvcfg |= ENVCFG_CBIE | ENVCFG_CBCFE;
        // ZICBOZ
        self.cfg.henvcfg |= ENVCFG_CBZE;
        // SVADU & SVADE
        self.cfg.henvcfg |= ENVCFG_ADUE;
        // guest_debug
        // self.cfg.hedeleg &= !(1<<3);
    }

    /// 1
    pub fn update_hvip(&self) {
        let hvip = self.guest_csr.hvip;
        unsafe {
            Hvip::write(hvip);
        }
    }

    /// 1
    pub fn swap_in_guest_state(&mut self){
        let scounteren = self.guest_csr.scounteren;
        self.host_scounteren = riscv::register::scounteren::read().bits();
        unsafe {
            riscv::register::scounteren::write(riscv::register::scounteren::Scounteren::from_bits(scounteren));
        }
        let senvcfg = self.guest_csr.senvcfg;
        self.host_senvcfg = riscv::register::senvcfg::read().bits();
        unsafe {
            riscv::register::senvcfg::write(riscv::register::senvcfg::Senvcfg::from_bits(senvcfg));
        }
    }

    /// 1
    pub fn swap_in_host_state(&mut self) {
        let scounteren = self.host_scounteren;
        self.guest_csr.scounteren = riscv::register::scounteren::read().bits();
        unsafe {
            riscv::register::scounteren::write(riscv::register::scounteren::Scounteren::from_bits(scounteren));
        }
        let senvcfg = self.host_senvcfg;
        self.guest_csr.senvcfg = riscv::register::senvcfg::read().bits();
        unsafe {
            riscv::register::senvcfg::write(riscv::register::senvcfg::Senvcfg::from_bits(senvcfg));
        }
    }

    /// 1
    pub fn enter_exit(&mut self) -> KvmCpuTrap {
        unsafe {
            self.host_context.set_hstatus(Hstatus::swap(self.guest_context.get_hstatus()));

            __kvm_riscv_switch_to(self as *mut _);

            self.guest_context.set_hstatus(Hstatus::swap(self.host_context.get_hstatus()));

            KvmCpuTrap {
                sepc: self.guest_context.get_sepc(),
                scause: riscv::register::scause::read().bits(),
                stval: riscv::register::stval::read(),
                htval: Htval::read(),
                htinst: Htinst::read(),
            }
        }
    }

    /// 1
    pub fn sync_interrupts(&mut self) {
        unsafe {
            self.guest_csr.vsie = Vsie::read();

            let hvip = Hvip::read();

            if (self.guest_csr.hvip ^ hvip) & (0x1 << 2) != 0 {
                if hvip & (0x1 << 2) != 0 {
                    let should_set = (self.irqs_pending_mask & 0x1 << 2) == 0; 
                    self.irqs_pending_mask |= 0x1 << 2;
                    if should_set {
                        self.irqs_pending |= 0x1 << 2;
                    }
                } else {
                    let should_clear = (self.irqs_pending_mask & 0x1 << 2) == 0; 
                    self.irqs_pending_mask |= 0x1 << 2;
                    if should_clear {
                        self.irqs_pending &= !(0x1 << 2);
                    }
                }
            }
            
            self.timer.sync();
        }
    }

    /// 1
    pub fn sbi_call(&mut self) -> Result<usize> {
        handle_sbi_call(self);
        Ok(1)
    }

    /// 1
    pub fn config_load(&self) {
        unsafe {
            Hedeleg::write(self.cfg.hedeleg);
            Hideleg::write(self.cfg.hideleg);
            Henvcfg::write(self.cfg.henvcfg);
        }
    }

    /// 1
    pub fn update_hgatp(&self, pgd_paddr: u64) {
        let mut hgatp = 0x9 << 60;
        hgatp |= pgd_paddr >> 12 & (0xFFFFFFFFFFF);
        unsafe {
            Hgatp::write(hgatp as usize);
        }
    }

    fn timer_restore(&mut self) {
        self.timer.restore();
    }

    /// 1
    pub fn flush_interrupts(&mut self) {
        let mask = self.irqs_pending_mask;
        if mask != 0 {
            self.irqs_pending_mask = 0;
            let val = self.irqs_pending & mask;

            self.guest_csr.hvip &= !mask as usize;
            self.guest_csr.hvip |= val as usize;
        }
    }

    /// 1
    pub fn vcpu_load(&mut self, pgd_paddr: u64) {
        self.config_load();

        unsafe {
            Vsstatus::write(self.guest_csr.vsstatus);
            Vsie::write(self.guest_csr.vsie);
            Vstvec::write(self.guest_csr.vstvec);
            Vsscratch::write(self.guest_csr.vsscratch);
            Vsepc::write(self.guest_csr.vsepc);
            Vscause::write(self.guest_csr.vscause);
            Vstval::write(self.guest_csr.vstval);
            Hvip::write(self.guest_csr.hvip);
            Vsatp::write(self.guest_csr.vsatp);
        }

        self.update_hgatp(pgd_paddr);

        self.timer_restore();

        // TODO: add fpu
    }

    fn timer_save(&mut self) {
        unsafe {
            Vstimecmp::write(usize::MAX);
        }
    }

    /// 1
    pub fn vcpu_put(&mut self) {
        self.timer_save();

        unsafe {
            self.guest_csr.vsstatus = Vsstatus::read();
            self.guest_csr.vsie = Vsie::read();
            self.guest_csr.vstvec = Vstvec::read();
            self.guest_csr.vsscratch = Vsscratch::read();
            self.guest_csr.vsepc = Vsepc::read();
            self.guest_csr.vscause = Vscause::read();
            self.guest_csr.vstval = Vstval::read();
            self.guest_csr.hvip = Hvip::read();
            self.guest_csr.vsatp = Vsatp::read();
        }
    }

    /// 1
    pub fn set_interrupt(&mut self, irq: u32) {
        if irq == u32::MAX {
            self.irqs_pending |= 0x1<<10;
        } else {
            self.irqs_pending &= !(0x1<<10);
        }
        self.irqs_pending_mask |= 0x1 <<10;
    }

    /// 1
    pub fn get_gatp(&self) -> usize {
        unsafe{Hgatp::read()}
    }
}

unsafe extern "C" {
    // We only use this pointer to read/write guest_context/host_context.
    // So we can use #[allow(improper_ctypes)] to mask the parts of the struct 
    // that are not FFI-safe and would otherwise trigger warnings.
    #[allow(improper_ctypes)]
    unsafe fn __kvm_riscv_switch_to(vcpu: *mut VcpuArchInner);
}
