use ostd::{arch::vm::GuestCpuConfig, vm::GuestPhysMemSpace};

use super::{
    apic::{IOAPIC_NUM_PINS, Icr, Ioapic, Lapic, default_lapic_ldr, icr_matches_destination},
    vcpu::Vcpu,
};
use crate::prelude::*;

pub(super) struct Vm {
    pub(super) id: u32,
    guest_mem: GuestPhysMemSpace,
    vcpus: Mutex<BTreeMap<u32, Arc<Vcpu>>>,
    ioapic: Mutex<Ioapic>,
}

impl Vm {
    pub fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            guest_mem: GuestPhysMemSpace::new(),
            vcpus: Mutex::new(BTreeMap::new()),
            ioapic: Mutex::new(Ioapic::default()),
        })
    }

    pub fn ioapic(&self) -> MutexGuard<'_, Ioapic> {
        self.ioapic.lock()
    }

    pub fn guest_mem(&self) -> &GuestPhysMemSpace {
        &self.guest_mem
    }

    pub(super) fn create_vcpu(self: &Arc<Self>, vcpu_id: u32) -> Result<Arc<Vcpu>> {
        let mut lapic = Lapic::default();
        lapic.id = vcpu_id;
        lapic.ldr = default_lapic_ldr(vcpu_id);

        let vcpu = Vcpu::new(vcpu_id, self, lapic)?;
        self.vcpus.lock().insert(vcpu_id, vcpu.clone());
        self.refresh_guest_cpu_config();
        Ok(vcpu)
    }

    fn refresh_guest_cpu_config(&self) {
        let vcpus = self.vcpus.lock();
        let vcpu_count = vcpus.len() as u32;
        for (&vcpu_id, vcpu) in vcpus.iter() {
            vcpu.guest_context().set_guest_cpu_config(GuestCpuConfig {
                vcpu_id,
                lapic_id: vcpu_id,
                vcpu_count,
            });
        }
    }

    fn inject_irq_line(&self, irq: usize) -> Result<()> {
        if irq >= IOAPIC_NUM_PINS {
            return_errno_with_message!(
                Errno::EINVAL,
                "IRQ line is out of range for the emulated I/O APIC"
            );
        }

        let vcpus = self.vcpus.lock().values().cloned().collect::<Vec<_>>();
        if vcpus.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "cannot inject IRQ without any vCPU");
        }

        let mut lapics = vcpus.iter().map(|vcpu| vcpu.lapic()).collect::<Vec<_>>();
        let mut ioapic = self.ioapic.lock();
        ioapic.inject_irq_line(lapics.iter_mut().map(|lapic| &mut **lapic), irq);
        Ok(())
    }

    pub fn inject_ipi(&self, icr: Icr) -> Result<()> {
        let vcpus: alloc::vec::Vec<_> = self
            .vcpus
            .lock()
            .iter()
            .map(|(&vcpu_id, vcpu)| (vcpu_id, vcpu.clone()))
            .collect();

        for (vcpu_id, vcpu) in vcpus {
            if !icr_matches_destination(&vcpu.lapic(), &icr) {
                continue;
            }

            const APIC_ICR_DELIVERY_MODE_FIXED: u8 = 0;
            const APIC_ICR_DELIVERY_MODE_INIT: u8 = 5;
            const APIC_ICR_DELIVERY_MODE_STARTUP: u8 = 6;

            match icr.delivery_mode {
                APIC_ICR_DELIVERY_MODE_FIXED => {
                    if icr.vector >= 16 {
                        vcpu.lapic().add_pending_interrupt(icr.vector);
                    }
                }
                APIC_ICR_DELIVERY_MODE_INIT => {
                    // Vcpu recieves INIT IPI, do nothing.
                    warn!(
                        "rustshyper: INIT IPI for vcpu {} is not fully migrated yet",
                        vcpu_id
                    );
                }
                APIC_ICR_DELIVERY_MODE_STARTUP => vcpu.receive_sipi(icr.vector),
                _ => {
                    error!(
                        "rustshyper: unsupported LAPIC ICR delivery mode {}",
                        icr.delivery_mode,
                    );
                }
            }
        }

        Ok(())
    }
}
