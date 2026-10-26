// SPDX-License-Identifier: MPL-2.0

use riscv::register::{scounteren, senvcfg};

use super::{
    GuestContext,
    context_switch::vcpu_run,
    exit::{GuestExitInfo, exit_info},
    // GuestExitInfo, VcpuArchState, VmxExitReason, exit,
};
use crate::{
    arch::vm::{
        csr::*,
        exit::{
            INTERRUPT_BIT, SUPERVISOR_EXTERNAL_INTERRUPT, SUPERVISOR_SOFTWARE_INTERRUPT,
            SUPERVISOR_TIMER_INTERRUPT,
        },
    },
    irq::{DisabledLocalIrqGuard, disable_local},
    prelude::*,
    vm::{GuestModeHooks, GuestPhysMemSpace},
};

/// An execution object.
pub(crate) struct ArchGuestMode;

impl ArchGuestMode {
    /// Creates a guest execution object without entering a guest.
    pub(crate) fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Attempts one guest entry and returns an exit that needs higher-level handling.
    ///
    /// Returns `None` for an internally handled exit, with local IRQs enabled.
    ///
    /// # Panics
    ///
    /// Local IRQs must be enabled.
    pub(crate) fn execute_once<H: GuestModeHooks + ?Sized>(
        &self,
        context: &mut GuestContext,
        guest_mem: &GuestPhysMemSpace,
        hooks: &H,
    ) -> Result<Option<GuestExitInfo>> {
        // 获取 irq_guard
        let irq_guard = disable_local();

        let arch = context.arch_mut();

        hooks.pre_guest_run(arch, &irq_guard);

        let csrs = &arch.csrs;
        // vCSR / Hedeleg Hideleg Henvcfg
        // SAFETY: Guest entry runs in HS-mode with the H extension enabled.
        // Local IRQs are disabled while installing the current guest's CSR state.
        unsafe {
            Hedeleg::write(csrs.hedeleg);
            Hideleg::write(csrs.hideleg);
            Henvcfg::write(csrs.henvcfg);

            Vsstatus::write(csrs.vsstatus);
            Vsie::write(csrs.vsie);
            Vstvec::write(csrs.vstvec);
            Vsscratch::write(csrs.vsscratch);
            Vsepc::write(csrs.vsepc);
            Vscause::write(csrs.vscause);
            Vstval::write(csrs.vstval);
            Hvip::write(csrs.hvip);
            Vsatp::write(csrs.vsatp);
        }

        // 保存 host 的 hstatus scounteren senvcfg，设置 guest 的 hstatus scounteren senvcfg
        let host_restorer =
            HostRestorer::new(&irq_guard, csrs.hstatus, csrs.scounteren, csrs.senvcfg);

        // 设置 hgatp
        let mut hgatp = 0x9 << 60;
        hgatp |= guest_mem.root_paddr() >> 12 & (0x0FFF_FFFF_FFFF);
        unsafe {
            Hgatp::write(hgatp as usize);
        }

        let (regs, csrs) = (&mut arch.regs, &mut arch.csrs);
        unsafe { vcpu_run(regs, csrs, &irq_guard) };

        // 设置 host 的 hstatus scounteren senvcfg，保存 guest 的 hstatus scounteren senvcfg
        let csrs = &mut arch.csrs;
        csrs.scounteren = scounteren::read().bits();
        csrs.senvcfg = senvcfg::read().bits();
        drop(host_restorer);

        // Save guest CSRs before clearing interrupt and delegation state.
        // SAFETY: Execution has returned to HS-mode with the H extension enabled,
        // and local IRQs remain disabled while saving and clearing guest CSR state.
        unsafe {
            csrs.vsstatus = Vsstatus::read();
            csrs.vsie = Vsie::read();
            csrs.vstvec = Vstvec::read();
            csrs.vsscratch = Vsscratch::read();
            csrs.vsepc = Vsepc::read();
            csrs.vscause = Vscause::read();
            csrs.vstval = Vstval::read();
            csrs.hvip = Hvip::read();
            csrs.vsatp = Vsatp::read();

            Vsie::write(0);
            Hvip::write(0);
            Hedeleg::write(0);
            Hideleg::write(0);
            Henvcfg::write(0);
        }

        hooks.post_guest_run(arch, &irq_guard);

        // 获取 vmexit 信息
        let exit = exit_info(&arch.csrs);

        // HSMode 中断由 host 处理
        drop(irq_guard);

        // 部分退出快速在 ostd 内处理
        let is_interrupt = exit.scause & INTERRUPT_BIT != 0;
        let code = exit.scause & !INTERRUPT_BIT;
        let is_host_irq = is_interrupt
            && matches!(
                code,
                SUPERVISOR_SOFTWARE_INTERRUPT
                    | SUPERVISOR_TIMER_INTERRUPT
                    | SUPERVISOR_EXTERNAL_INTERRUPT
            );

        if is_host_irq {
            Ok(None)
        } else {
            Ok(Some(exit))
        }
    }
}

/// Restores host CSRs before local IRQs can be re-enabled.
///
/// Guest CSR values must be saved separately before this guard is dropped.
#[must_use]
struct HostRestorer<'a> {
    scounteren: usize,
    senvcfg: usize,
    hstatus: usize,
    _irq_guard: &'a DisabledLocalIrqGuard,
}

impl<'a> HostRestorer<'a> {
    fn new(
        irq_guard: &'a DisabledLocalIrqGuard,
        guest_hstatus: usize,
        guest_scounteren: usize,
        guest_senvcfg: usize,
    ) -> Self {
        // SAFETY:
        unsafe {
            let restorer = Self {
                scounteren: scounteren::read().bits(),
                senvcfg: senvcfg::read().bits(),
                hstatus: Hstatus::swap(guest_hstatus),
                _irq_guard: irq_guard,
            };

            scounteren::write(scounteren::Scounteren::from_bits(guest_scounteren));
            senvcfg::write(senvcfg::Senvcfg::from_bits(guest_senvcfg));

            restorer
        }
    }
}

impl Drop for HostRestorer<'_> {
    fn drop(&mut self) {
        // SAFETY:
        unsafe {
            Hstatus::write(self.hstatus);
            scounteren::write(scounteren::Scounteren::from_bits(self.scounteren));
            senvcfg::write(senvcfg::Senvcfg::from_bits(self.senvcfg));
            // Hvip::write(0);
            // Vsie::write(0);
        }
    }
}
