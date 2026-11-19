// SPDX-License-Identifier: MPL-2.0

//! The PCI bus in the device model.
//!
//! [`PciBus`] uses [`PciCommonDevice`] as its device payload.
//! Drivers registered on this bus must provide a slice of [`PciMatch`] rules for device matching.
//! The bus exposes read-only `vendor`, `device`, `class`, and `revision` sysfs attributes on each [`PciDevice`].
//! Its uevent environment includes `PCI_CLASS`, `PCI_ID`, and `PCI_SLOT_NAME`.

use alloc::sync::Arc;

use aster_device::{
    bus::{Bus, BusDevice},
    common::Attr,
    uevent::UeventVars,
};
use ostd::Result;

use crate::{capability::msix::CapabilityMsixData, common_device::PciCommonDevice};

/// The PCI bus.
pub struct PciBus;

/// A device on the PCI bus.
pub type PciDevice = BusDevice<PciBus>;

/// PCI-specific operations on a device handle.
pub trait PciDeviceExt {
    /// Acquires necessary resources to build the MSI-X capability data, if the capability exists.
    ///
    /// Returns `Ok(None)` if the device has no MSI-X capability.
    /// Dropping the returned capability disables its interrupts, releases its IRQ lines,
    /// and allows it to be acquired again.
    ///
    /// # Errors
    ///
    /// - [`InvalidArgs`](ostd::Error::InvalidArgs) if the MSI-X table or pending bit array
    ///   references a missing BAR or an I/O BAR instead of a memory BAR.
    /// - [`AccessDenied`](ostd::Error::AccessDenied) if the required BAR I/O memory cannot
    ///   be acquired, or the MSI-X capability is already acquired.
    fn acquire_msix_capability(self: &Arc<Self>) -> Result<Option<CapabilityMsixData>>;
}

impl PciDeviceExt for PciDevice {
    fn acquire_msix_capability(self: &Arc<Self>) -> Result<Option<CapabilityMsixData>> {
        CapabilityMsixData::new(self)
    }
}

/// Matching criteria for PCI drivers.
#[derive(Clone, Copy, Debug)]
pub enum PciMatch {
    /// Matches a vendor and device IDs from `first` to `last`, both inclusive.
    Device { vendor: u16, first: u16, last: u16 },
    /// Matches a class, subclass, and programming interface.
    Class {
        class: u8,
        subclass: u8,
        prog_if: u8,
    },
}

impl Bus for PciBus {
    const NAME: &'static str = "pci";

    type Device = PciCommonDevice;
    type MatchData = [PciMatch];

    fn matches(&self, dev: &PciCommonDevice, rules: &Self::MatchData) -> bool {
        let id = dev.device_id();
        rules.iter().any(|rule| match *rule {
            PciMatch::Device {
                vendor,
                first,
                last,
            } => id.vendor_id == vendor && (first..=last).contains(&id.device_id),
            PciMatch::Class {
                class,
                subclass,
                prog_if,
            } => id.class == class && id.subclass == subclass && id.prog_if == prog_if,
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<PciDevice>] {
        const ATTRS: &[Attr<PciDevice>] = &[
            Attr::ro("vendor", |dev, w| {
                writeln!(w, "0x{:04x}", dev.device_id().vendor_id)?;
                Ok(())
            }),
            Attr::ro("device", |dev, w| {
                writeln!(w, "0x{:04x}", dev.device_id().device_id)?;
                Ok(())
            }),
            Attr::ro("class", |dev, w| {
                let id = dev.device_id();
                writeln!(w, "0x{:02x}{:02x}{:02x}", id.class, id.subclass, id.prog_if)?;
                Ok(())
            }),
            Attr::ro("revision", |dev, w| {
                writeln!(w, "0x{:02x}", dev.device_id().revision_id)?;
                Ok(())
            }),
        ];
        ATTRS
    }

    fn append_uevent_vars(
        &self,
        dev: &PciDevice,
        vars: &mut UeventVars,
    ) -> aster_device::common::Result<()> {
        // Match Linux's PCI uevent formatting for these variables:
        // <https://elixir.bootlin.com/linux/v6.16/source/drivers/pci/pci-driver.c#L1549>.
        // Omit `PCI_SUBSYS_ID` and `MODALIAS`
        // because the PCI device model does not yet track subsystem IDs.

        let id = dev.device_id();
        let class =
            (u32::from(id.class) << 16) | (u32::from(id.subclass) << 8) | u32::from(id.prog_if);
        let location = dev.location();

        vars.add("PCI_CLASS", format_args!("{class:04X}"))?;
        vars.add(
            "PCI_ID",
            format_args!("{:04X}:{:04X}", id.vendor_id, id.device_id),
        )?;
        vars.add(
            "PCI_SLOT_NAME",
            format_args!(
                "0000:{:02x}:{:02x}.{}",
                location.bus, location.device, location.function
            ),
        )?;

        Ok(())
    }
}
