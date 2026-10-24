// SPDX-License-Identifier: MPL-2.0

//! Driver for the Amazon Elastic Network Adapter (ENA), the NIC of EC2
//! Nitro instances (PCI `1d0f:ec20` and friends).
//!
//! Scope: up to `ena.queues=` (default: one per vCPU, max 8) Tx/Rx queue
//! pairs in host memory with RSS over the IPv4 5-tuple, descriptor-based
//! completions, TCP/UDP checksum offload on Tx (partial, pseudo-header
//! supplied) and Rx, MSI-X interrupts per queue pair, a polled admin queue,
//! and AENQ handling (keep-alive watchdog, link change, fatal error) with a
//! full device reset path. No LLQ, no TSO. The structure follows
//! `ena_com.c` so the remaining pieces can be added next to their Linux
//! counterparts.
//!
//! Command-line knobs: `ena.queues=N`, `ena.test_reset=SECONDS` (force one
//! reset after boot, for testing the recovery path).
//!
//! Interrupt delivery on Nitro has not been exercised by every path of
//! this kernel yet, so the driver also raises the network softirqs from
//! the timer tick (every 4 ms). Both paths are idempotent; the tick only
//! bounds the latency if an MSI-X message is lost.
//!
//! Verbose per-packet logging is at the debug level (`loglevel=debug`).

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "ena: "
    };
}

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::Ordering;

use aster_pci::{
    PCI_BUS, PciDeviceId,
    bus::{PciDevice, PciDriver},
    capability::msix::CapabilityMsixData,
    cfg_space::{Bar, BarAccess},
    common_device::PciCommonDevice,
};
use component::{ComponentInitError, init_component};
use ostd::{bus::BusProbeError, sync::SpinLock};
use spin::Once;

mod admin;
mod device;
mod io;
mod regs;

pub use device::{DEVICE_NAME, EnaDevice};

const ENA_VENDOR: u16 = 0x1d0f;
/// PF, LLQ PF, VF, LLQ VF.
const ENA_DEVICES: [u16; 4] = [0x0ec2, 0x1ec2, 0xec20, 0xec21];

/// What `probe` collects for every claimed ENA function, consumed by `init`.
struct Probed {
    bar0: BarAccess,
    msix: CapabilityMsixData,
    id: PciDeviceId,
}

#[derive(Debug)]
struct EnaPciDevice(PciDeviceId);

impl PciDevice for EnaPciDevice {
    fn device_id(&self) -> PciDeviceId {
        self.0
    }
}

#[derive(Debug)]
struct EnaPciDriver {
    probed: SpinLock<Vec<Probed>>,
    /// Every unclaimed function we were offered and declined, for diagnostics.
    declined: SpinLock<Vec<(PciDeviceId, aster_pci::PciDeviceLocation)>>,
}

impl core::fmt::Debug for Probed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Probed")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl PciDriver for EnaPciDriver {
    fn probe(
        &self,
        mut device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        let id = *device.device_id();
        if id.vendor_id != ENA_VENDOR || !ENA_DEVICES.contains(&id.device_id) {
            self.declined.lock().push((id, *device.location()));
            return Err((BusProbeError::DeviceNotMatch, device));
        }
        ostd::early_println!(
            "[kernel] ena: found {:04x}:{:04x} rev {} at {:?}",
            id.vendor_id,
            id.device_id,
            id.revision_id,
            device.location()
        );

        let bar0 = match device.bar_manager_mut().bar_mut(0) {
            Some(bar @ Bar::Memory(_)) => match bar {
                Bar::Memory(mem) if mem.size() < regs::BAR0_MIN_SIZE => {
                    ostd::error!("BAR0 too small ({} bytes)", mem.size());
                    return Err((BusProbeError::ConfigurationSpaceError, device));
                }
                _ => match bar.acquire() {
                    Ok(access) => access,
                    Err(e) => {
                        ostd::error!("cannot map BAR0: {:?}", e);
                        return Err((BusProbeError::ConfigurationSpaceError, device));
                    }
                },
            },
            _ => {
                ostd::error!("BAR0 missing or not MMIO");
                return Err((BusProbeError::ConfigurationSpaceError, device));
            }
        };
        let msix = match device.acquire_msix_capability() {
            Ok(Some(msix)) => msix,
            Ok(None) => {
                ostd::error!("no MSI-X capability");
                return Err((BusProbeError::ConfigurationSpaceError, device));
            }
            Err(e) => {
                ostd::error!("MSI-X setup failed: {:?}", e);
                return Err((BusProbeError::ConfigurationSpaceError, device));
            }
        };
        // Allocate an IRQ line for every table entry so that `irq_mut` works.
        let mut msix = msix;
        for i in 0..msix.table_size() {
            match ostd::irq::IrqLine::alloc() {
                Ok(irq) => msix.set_interrupt_vector(irq, i),
                Err(e) => {
                    ostd::error!("cannot allocate IRQ for MSI-X entry {}: {:?}", i, e);
                    break;
                }
            }
        }
        ostd::debug!("MSI-X table size {}", msix.table_size());

        self.probed.lock().push(Probed { bar0, msix, id });
        Ok(Arc::new(EnaPciDevice(id)))
    }
}

static DRIVER: Once<Arc<EnaPciDriver>> = Once::new();

pub(crate) static QUEUES_PARAM: Once<alloc::string::String> = Once::new();
aster_cmdline::define_kv_param!("ena.queues", QUEUES_PARAM);
pub(crate) static TEST_RESET_PARAM: Once<alloc::string::String> = Once::new();
aster_cmdline::define_kv_param!("ena.test_reset", TEST_RESET_PARAM);
/// `ena.offload=0` disables Tx/Rx checksum offload (software checksums).
pub(crate) static OFFLOAD_PARAM: Once<alloc::string::String> = Once::new();
aster_cmdline::define_kv_param!("ena.offload", OFFLOAD_PARAM);
/// `ena.aenq_irq=1` unmasks the admin interrupt (default: masked, AENQ polled every 100 ms).
pub(crate) static AENQ_IRQ_PARAM: Once<alloc::string::String> = Once::new();
aster_cmdline::define_kv_param!("ena.aenq_irq", AENQ_IRQ_PARAM);

/// Fallback poll: raise the network softirqs every `TICK_DIVIDER` timer ticks.
const TICK_DIVIDER: u32 = 4;
/// Device health (AENQ, keep-alive) every `HEALTH_DIVIDER` ticks.
const HEALTH_DIVIDER: u32 = 100;

#[init_component]
fn ena_init() -> Result<(), ComponentInitError> {
    device::init_pools();
    let driver = Arc::new(EnaPciDriver {
        probed: SpinLock::new(Vec::new()),
        declined: SpinLock::new(Vec::new()),
    });
    DRIVER.call_once(|| driver.clone());
    PCI_BUS.lock().register_driver(driver.clone());

    let probed: Vec<Probed> = core::mem::take(&mut *driver.probed.lock());
    if probed.is_empty() {
        // Visible at every log level: on EC2 this is the line that explains "no network".
        let declined = driver.declined.lock();
        ostd::early_println!(
            "[kernel] ena: no ENA function among the {} unclaimed PCI functions:",
            declined.len()
        );
        for (id, loc) in declined.iter() {
            ostd::early_println!(
                "[kernel] ena:   {:02x}:{:02x}.{} {:04x}:{:04x} class {:02x}/{:02x}/{:02x}",
                loc.bus,
                loc.device,
                loc.function,
                id.vendor_id,
                id.device_id,
                id.class,
                id.subclass,
                id.prog_if
            );
        }
    }
    let mut registered = 0;
    for p in probed {
        let admin = match admin::AdminQueue::new(p.bar0) {
            Ok(a) => a,
            Err(e) => {
                ostd::early_println!(
                    "[kernel] ena: {:04x}:{:04x}: admin queue init failed: {:?}",
                    p.id.vendor_id,
                    p.id.device_id,
                    e
                );
                continue;
            }
        };
        match EnaDevice::init(admin, p.msix) {
            Ok(()) => registered += 1,
            Err(e) => ostd::early_println!(
                "[kernel] ena: {:04x}:{:04x}: device init failed: {:?}",
                p.id.vendor_id,
                p.id.device_id,
                e
            ),
        }
        // One interface is all the network stack binds to today.
        break;
    }
    if registered > 0 {
        ostd::timer::register_callback_on_cpu(|| {
            device::TICK_MS.fetch_add(1, Ordering::Relaxed);
            let t = device::TICKS.fetch_add(1, Ordering::Relaxed);
            if t.is_multiple_of(TICK_DIVIDER) {
                aster_network::raise_receive_softirq();
                aster_network::raise_send_softirq();
            }
            if t.is_multiple_of(HEALTH_DIVIDER) || device::AENQ_PENDING.load(Ordering::Acquire) {
                device::TICK_DUE.store(true, Ordering::Release);
                aster_network::raise_receive_softirq();
            }
        });
    }
    Ok(())
}
