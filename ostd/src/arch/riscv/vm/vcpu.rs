// SPDX-License-Identifier: MPL-2.0

use super::{
    context::{KvmVcpuConfig, KvmVcpuCsr, VcpuContext},
    timer::VcpuTimer,
};

use crate::arch::cpu::extension::{IsaExtensions, has_extensions};

use super::exit::GuestExitInfo;
use crate::mm::PAGE_SIZE;
use super::csr::{HstatusFlags, HenvcfgFlags, Hvip, Hstatus, Vsie, Hedeleg, Hideleg, Henvcfg, Hgatp, Vsstatus, Vstvec, Vsscratch, Vsepc, Vscause, Vstval, Vsatp};
use super::types::{KVM_HEDELEG_DEFAULT, KVM_HIDELEG_DEFAULT, InterruptCauses, HGATP_MODE_LEVEL_48, HGATP_MODE_SHIFT, HGATP_PPN};
use riscv::register::sstatus::{Sstatus, SPP, FS};

/// The state of a vCPU, including registers, interrupts, and timer.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VcpuArchState {
    /// Vendor details.
    pub mvendorid: usize,
    /// Arch details.
    pub marchid: usize,
    /// Implementation details.
    pub mimpid: usize,

    /// SSCRATCH, STVEC, SCOUNTEREN, SENVCFG, and SSTATEEN0 of Host.
    pub(super) host_sscratch: usize,
    pub(super) host_stvec: usize,
    pub(super) host_scounteren: usize,
    pub(super) host_senvcfg: usize,
    pub(super) host_sstateen0: usize,

    /// CPU context of Host.
    pub host_context: VcpuContext,

    /// CPU context of Guest VCPU.
    pub guest_context: VcpuContext,

    /// CPU CSR context of Guest VCPU.
    pub guest_csr: KvmVcpuCsr,

    /// 'static' configurations which are set only once
    cfg: KvmVcpuConfig,

    /// ### VCPU interrupts
    /// The `irqs_pending` bitmap represents pending interrupts,
    /// whereas `irqs_pending_mask` represents bits changed in `irqs_pending`.
    /// Updates to these bitmaps are serialized,
    /// so interrupt sync/flush cannot drop a newly injected interrupt
    /// while syncing guest-visible HVIP.
    ///
    /// Note: If we do not use AIA to implement interrupt virtualization,
    /// all guest interrupts must originate from the user-space VMM (e.g., QEMU) via `SET_INTERRUPT`.
    /// However, the VMM's invocation of the `ioctl` requires CPU resources;
    /// this means that in a single-core environment,
    /// unless the `vcpu_run` thread yields control,
    /// the guest will never receive the interrupt.
    ///
    /// Note: Generally, interrupts are associated with specific interrupt numbers,
    /// however, we do not need to concern ourselves with the specific number;
    /// only with whether an interrupt has occurred.
    /// This is why `set_interrupt` does not distinguish between interrupt numbers.
    /// We only need to tell the guest that an interrupt is pending;
    /// the guest then queries the PLIC via MMIO to find the specific number.
    /// Since the PLIC is managed by the user-space VMM (e.g. QEMU),
    /// all the necessary setup has already been handled.
    irqs_pending: u64,
    irqs_pending_mask: u64,

    /// VCPU Timer.
    pub timer: VcpuTimer,
}

impl VcpuArchState {
    /// Init the vcpu.
    pub fn init(&mut self){
        // Setup VCPU config.
        self.cfg.hedeleg = KVM_HEDELEG_DEFAULT;
        self.cfg.hideleg = KVM_HIDELEG_DEFAULT;

        // Setup vendor, arch, and implementation details.
	    self.mvendorid = sbi_rt::get_mvendorid();
	    self.marchid = sbi_rt::get_marchid();
	    self.mimpid = sbi_rt::get_mimpid();
    }

    /// Reset the context of Vcpu.
    pub fn context_reset(&mut self) {
        self.guest_context = VcpuContext::default();
        self.guest_csr = KvmVcpuCsr::default();

        // Setup reset state of shadow SSTATUS and HSTATUS CSRs.
        let mut sstatus = Sstatus::from_bits(0);
        sstatus.set_spp(SPP::Supervisor);
        sstatus.set_spie(true);
        self.guest_context.sstatus = sstatus.bits();

        // SAFETY: All valid bits have been defined; the remaining bits are valid reserved bits.
        let mut hstatus = unsafe { HstatusFlags::from_bits_unchecked(self.guest_context.hstatus) };
        hstatus |= HstatusFlags::VTW | HstatusFlags::SPVP | HstatusFlags::SPV;
        self.guest_context.hstatus = hstatus.bits();
    }

    /// Reset the context of float-extension.
    pub fn fp_reset(&mut self) {
        let mut sstatus = Sstatus::from_bits(self.guest_context.sstatus);

        if has_extensions(IsaExtensions::F) || has_extensions(IsaExtensions::D) {
            // Set FS=Initial to allow guest use fp_reg and fp_insn.
            sstatus.set_fs(FS::Initial);
        } else {
            sstatus.set_fs(FS::Off);
        }

        self.guest_context.sstatus = sstatus.bits();
    }

    /// Set config regs to vcpu which has ran once.
    pub fn config_ran_once(&mut self) {
        // SAFETY: All valid bits have been defined; the remaining bits are valid reserved bits.
        let mut henvcfg = unsafe { HenvcfgFlags::from_bits_unchecked(self.cfg.henvcfg) };
        // SSTC
        henvcfg |= HenvcfgFlags::STCE;
        // ZICBOM
        henvcfg |= HenvcfgFlags::CBIE | HenvcfgFlags::CBCFE;
        // ZICBOZ
        henvcfg |= HenvcfgFlags::CBZE;
        // SVADU & SVADE
        henvcfg |= HenvcfgFlags::ADUE;
        self.cfg.henvcfg = henvcfg.bits();
    }

    /// Update vcpu's hvip reg
    pub(super) fn update_hvip(&self) {
        // SAFETY: At the time of the call, 
        // it is guaranteed that interrupts have been synchronized and the system is about to enter the guest.
        unsafe { Hvip::write(self.guest_csr.hvip); }
    }

    /// Save host csrs in host_context and load guest csrs
    fn swap_in_guest_state(&mut self){
        let scounteren = self.guest_csr.scounteren;
        self.host_scounteren = riscv::register::scounteren::read().bits();

        // SAFETY: Here, we are switching between host and guest CSRs; 
        // we have already ensured that the values ​​being written are valid.
        unsafe {
            riscv::register::scounteren::write(riscv::register::scounteren::Scounteren::from_bits(scounteren));
        }
        let senvcfg = self.guest_csr.senvcfg;
        self.host_senvcfg = riscv::register::senvcfg::read().bits();
        // SAFETY: Here, we are switching between host and guest CSRs; 
        // we have already ensured that the values ​​being written are valid.
        unsafe {
            riscv::register::senvcfg::write(riscv::register::senvcfg::Senvcfg::from_bits(senvcfg));
        }
    }

    /// Save guest csrs in guset_context and load host csrs
    fn swap_in_host_state(&mut self) {
        let scounteren = self.host_scounteren;
        self.guest_csr.scounteren = riscv::register::scounteren::read().bits();
        // SAFETY: Here, we are switching between host and guest CSRs; 
        // we have already ensured that the values ​​being written are valid.
        unsafe {
            riscv::register::scounteren::write(riscv::register::scounteren::Scounteren::from_bits(scounteren));
        }
        let senvcfg = self.host_senvcfg;
        self.guest_csr.senvcfg = riscv::register::senvcfg::read().bits();
        // SAFETY: Here, we are switching between host and guest CSRs; 
        // we have already ensured that the values ​​being written are valid.
        unsafe {
            riscv::register::senvcfg::write(riscv::register::senvcfg::Senvcfg::from_bits(senvcfg));
        }
    }

    /// Enter VM and will back to host.
    /// The trap info will be returned in GuestExitInfo
    pub(super) fn enter_exit(&mut self) -> GuestExitInfo {
        self.swap_in_guest_state();

        // SAFETY: Ensure that all operations are complete before making the call,
        // and update the Hstatus status register as the final step.
        unsafe { self.host_context.hstatus = Hstatus::swap(self.guest_context.hstatus); }
        
        // SAFETY: All operations requiring software intervention have been ensured to be complete;
        // the function reads the memory values ​​and stores them into the actual registers, 
        // after which all subsequent steps are carried out automatically by the hardware.
        unsafe {
            __kvm_riscv_switch_to(self as *mut _);
        }

        // SAFETY: Returned from guest; the `hstatus` state can now be switched.
        // and this operation precedes all subsequent actions.
        unsafe { self.guest_context.hstatus = Hstatus::swap(self.host_context.hstatus); }

        self.swap_in_host_state();

        self.exit_info()
    }

    /// We might get interrupts when we handle vcpu's trap in host(eg. in wfi_insn's yield).
    /// Now we should sync interrupts to vcpu after trap is handled.
    /// 
    /// Note: For details on how the host handles guest interrupts, 
    /// please refer to the comments for `irqs_pending`.
    pub(super) fn sync_interrupts(&mut self) {
        let hvip;

        // SAFETY: The guest execution state has been halted,
        // and the interrupt state has been fixed.
        unsafe {
            self.guest_csr.vsie = Vsie::read();
            hvip = Hvip::read();
        }

        let vssoft = InterruptCauses::VsSoft as usize;
        if (self.guest_csr.hvip ^ hvip) & (0x1 << vssoft) != 0 {
            if hvip & (0x1 << vssoft) != 0 {
                let should_set = (self.irqs_pending_mask & 0x1 << vssoft) == 0; 
                self.irqs_pending_mask |= 0x1 << vssoft;
                if should_set {
                    self.irqs_pending |= 0x1 << vssoft;
                }
            } else {
                let should_clear = (self.irqs_pending_mask & 0x1 << vssoft) == 0; 
                self.irqs_pending_mask |= 0x1 << vssoft;
                if should_clear {
                    self.irqs_pending &= !(0x1 << vssoft);
                }
            }
        }
            
        self.timer.sync();
    }
    
    fn config_load(&self) {
        // SAFETY: The call occurs before any guest operations are executed,
        // and it is guaranteed to be invoked exactly once per `ioctl` call.
        unsafe {
            Hedeleg::write(self.cfg.hedeleg);
            Hideleg::write(self.cfg.hideleg);
            Henvcfg::write(self.cfg.henvcfg);
        }
    }

    /// Update HGATP reg which is used in G-stage addr trans.
    pub(super) fn update_hgatp(&self, pgd_paddr: u64) {
        let mut hgatp = HGATP_MODE_LEVEL_48 << HGATP_MODE_SHIFT;
        hgatp |= pgd_paddr as usize >> PAGE_SIZE.trailing_zeros() & HGATP_PPN;
        // SAFETY: Ensure that all page table operations—including those for page faults—have been completed at the time of the call.
        unsafe { Hgatp::write(hgatp); }
    }

    /// Check whether the trap is from guest
    pub fn trap_is_from_guest(&self) -> bool {
        // SAFETY: All valid bits have been defined; the remaining bits are valid reserved bits.
        unsafe { HstatusFlags::from_bits_unchecked(self.guest_context.hstatus).contains(HstatusFlags::SPV) }
    }

    /// Now we have sync guest's interrupt in `VcpuArchState.irqs_pending`
    /// We should flush it to guest's csr regs before we enter VM.
    /// 
    /// Note: For details on how the host handles guest interrupts, 
    /// please refer to the comments for `irqs_pending`.
    pub(super) fn flush_interrupts(&mut self) {
        let mask = self.irqs_pending_mask;
        if mask != 0 {
            self.irqs_pending_mask = 0;
            let val = self.irqs_pending & mask;

            let mut hvip = self.guest_csr.hvip;
            hvip &= !mask as usize;
            hvip |= val as usize;
            self.guest_csr.hvip = hvip;
        }
    }

    /// Load Vcpu.
    /// 
    /// This operation must be performed before the guest actually begins execution.
    /// This function is not part of the `run` loop; in other words, 
    /// the number of times it is called should correspond to the number of times the `vcpu_run` ioctl is invoked.
    pub fn vcpu_load(&mut self) {
        self.config_load();

        // SAFETY: This call occurs before the execution of any guest operations and is invoked exactly once per `ioctl` call.
        // The subsequent switching of all registers is guaranteed to be handled automatically by the hardware.
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

        self.timer.restore();
    }

    /// Put Vcpu.
    /// 
    /// This operation must be performed when the guest execution ends and control returns to the user space.
    /// This function is not part of the `run` loop; in other words, 
    /// the number of times it is called should correspond to the number of times `ioctl vcpu_run` is invoked.
    pub fn vcpu_put(&mut self) {
        self.timer.save();

        // SAFETY: This invocation occurs after all guest operations have been completed and just before the transition back to user space;
        // it is triggered exactly once per `ioctl` call. The subsequent switching of all registers is handled automatically by the hardware.
        unsafe {
            self.guest_csr.vsstatus = Vsstatus::read();
            self.guest_csr.vsie=Vsie::read();
            self.guest_csr.vstvec=Vstvec::read();
            self.guest_csr.vsscratch=Vsscratch::read();
            self.guest_csr.vsepc=Vsepc::read();
            self.guest_csr.vscause=Vscause::read();
            self.guest_csr.vstval=Vstval::read();
            self.guest_csr.hvip=Hvip::read();
            self.guest_csr.vsatp=Vsatp::read();
        }
    }

    /// The VMM(from user-space, e.g. QEMU) told us guest has a interrupt.
    /// So we save the interrupt in `VcpuArchState.irqs_pending`.
    /// 
    /// Note: For details on how the host handles guest interrupts, 
    /// please refer to the comments for `irqs_pending`.
    pub fn set_interrupt(&mut self, irq: u32) {
        let vsext = InterruptCauses::VsExt as usize;
        if irq == u32::MAX {
            self.irqs_pending |= 0x1<<vsext;
        } else {
            self.irqs_pending &= !(0x1<<vsext);
        }
        self.irqs_pending_mask |= 0x1 <<vsext;
    }
}

unsafe extern "C" {
    unsafe fn __kvm_riscv_switch_to(vcpu: *mut VcpuArchState);
}
