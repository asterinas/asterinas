// SPDX-License-Identifier: MPL-2.0

//! The PCI bus in the device model.
//!
//! [`PciBus`] uses [`PciCommonDevice`] as its device payload.
//! Drivers registered on this bus must provide a slice of [`PciMatch`] rules for device matching.
//! The bus exposes read-only `vendor`, `device`, `class`, and `revision` sysfs attributes on each [`PciDevice`].

use aster_device::{
    bus::{Bus, BusDevice},
    common::Attr,
};

use crate::common_device::PciCommonDevice;

/// The PCI bus.
pub struct PciBus;

/// A device on the PCI bus.
pub type PciDevice = BusDevice<PciBus>;

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
}
