// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

//! Remapping structures of DMAR table.
//!
//! This file defines these structures and provides a `Debug` implementation to see the value
//! inside these structures.
//!
//! Most of the introduction are copied from Intel vt-directed-io-specification.

use alloc::{borrow::ToOwned, string::String, vec::Vec};
use core::fmt::Debug;

use ostd_pod::Pod;

/// DMA-remapping hardware unit definition (DRHD).
///
/// A DRHD structure uniquely represents a remapping hardware unit present in the platform.
/// There must be at least one instance of this structure for each PCI segment in the platform.
#[derive(Debug, Clone)]
pub struct Drhd {
    header: DrhdHeader,
    device_scopes: Vec<DeviceScope>,
}

impl Drhd {
    pub fn register_base_addr(&self) -> u64 {
        self.header.register_base_addr
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrhdHeader {
    typ: u16,
    length: u16,
    flags: u8,
    size: u8,
    segment_num: u16,
    register_base_addr: u64,
}

/// Reserved Memory Region Reporting (RMRR).
///
/// BIOS allocated reserved memory ranges that may be DMA targets.
/// It may report each such reserved memory region through the RMRR structures, along
/// with the devices that requires access to the specified reserved memory region.
#[derive(Debug, Clone)]
pub struct Rmrr {
    header: RmrrHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct RmrrHeader {
    typ: u16,
    length: u16,
    reserved: u16,
    segment_num: u16,
    reserved_memory_region_base_addr: u64,
    reserved_memory_region_limit_addr: u64,
}

/// Root Port ATS Capability Reporting (ATSR).
///
/// This structure is applicable only for platforms supporting Device-TLBs as reported through the
/// Extended Capability Register.
#[derive(Debug, Clone)]
pub struct Atsr {
    header: AtsrHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct AtsrHeader {
    typ: u16,
    length: u16,
    flags: u8,
    reserved: u8,
    segment_num: u16,
}

/// Remapping Hardware Status Affinity (RHSA).
///
/// It is applicable for platforms supporting non-uniform memory (NUMA),
/// where Remapping hardware units spans across nodes.
/// This optional structure provides the association between each Remapping hardware unit (identified
/// by its espective Base Address) and the proximity domain to which that hardware unit belongs.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct Rhsa {
    typ: u16,
    length: u16,
    flags: u32,
    register_base_addr: u64,
    proximity_domain: u32,
}

/// ACPI Name-space Device Declaration (ANDD).
///
/// An ANDD structure uniquely represents an ACPI name-space
/// enumerated device capable of issuing DMA requests in the platform.
#[derive(Debug, Clone)]
pub struct Andd {
    header: AnddHeader,
    acpi_object_name: String,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct AnddHeader {
    typ: u16,
    length: u16,
    reserved: [u8; 3],
    acpi_device_num: u8,
}

/// SoC Integrated Address Translation Cache (SATC).
///
/// The SATC reporting structure identifies devices that have address translation cache (ATC),
/// as defined by the PCI Express Base Specification.
#[derive(Debug, Clone)]
pub struct Satc {
    header: SatcHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SatcHeader {
    typ: u16,
    length: u16,
    flags: u8,
    reserved: u8,
    segment_num: u16,
}

/// SoC Integrated Device Property Reporting (SIDP).
///
/// The (SIDP) reporting structure identifies devices that have special
/// properties and that may put restrictions on how system software must configure remapping
/// structures that govern such devices in a platform where remapping hardware is enabled.
#[derive(Debug, Clone)]
pub struct Sidp {
    header: SidpHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct SidpHeader {
    typ: u16,
    length: u16,
    reserved: u16,
    segment_num: u16,
}

/// The Device Scope Structure is made up of Device Scope Entries. Each Device Scope Entry may be
/// used to indicate a PCI endpoint device
#[derive(Debug, Clone)]
pub struct DeviceScope {
    header: DeviceScopeHeader,
    path: Vec<(u8, u8)>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DeviceScopeHeader {
    typ: u8,
    length: u8,
    flags: u8,
    reserved: u8,
    enum_id: u8,
    start_bus_number: u8,
}

macro_rules! impl_from_bytes {
    ($(($struct:tt, $header_struct:tt),)*) => {
        $(impl $struct {
            #[doc = concat!("Parses a [`", stringify!($struct), "`] from bytes.")]
            ///
            /// # Panics
            ///
            #[doc = concat!(
                "This method may panic if the bytes do not represent a valid [`",
                stringify!($struct),
                "`].",
            )]
            pub fn from_bytes(bytes: &[u8]) -> Self {
                let header = $header_struct::from_bytes(bytes);
                debug_assert_eq!(header.length as usize, bytes.len());

                let mut index = core::mem::size_of::<$header_struct>();
                let mut device_scopes = Vec::new();
                while index != (header.length as usize) {
                    let val = DeviceScope::from_bytes_prefix(&bytes[index..]);
                    index += val.header.length as usize;
                    device_scopes.push(val);
                }

                Self{
                    header,
                    device_scopes,
                }
            }
        })*
    };
}

impl_from_bytes!(
    (Drhd, DrhdHeader),
    (Rmrr, RmrrHeader),
    (Atsr, AtsrHeader),
    (Satc, SatcHeader),
    (Sidp, SidpHeader),
);

impl DeviceScope {
    /// Parses a [`DeviceScope`] from a prefix of the bytes.
    ///
    /// # Panics
    ///
    /// This method may panic if the byte prefix does not represent a valid [`DeviceScope`].
    fn from_bytes_prefix(bytes: &[u8]) -> Self {
        let header = DeviceScopeHeader::from_bytes(bytes);
        debug_assert!((header.length as usize) <= bytes.len());

        let mut index = core::mem::size_of::<DeviceScopeHeader>();
        debug_assert!((header.length as usize) >= index);

        let mut path = Vec::new();
        while index != (header.length as usize) {
            let val = (bytes[index], bytes[index + 1]);
            path.push(val);
            index += 2;
        }

        Self { header, path }
    }
}

impl Rhsa {
    /// Parses an [`Rhsa`] from the bytes.
    ///
    /// # Panics
    ///
    /// This method may panic if the bytes do not represent a valid [`Rhsa`].
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let val = <Self as Pod>::from_bytes(bytes);
        debug_assert_eq!(val.length as usize, bytes.len());

        val
    }
}

impl Andd {
    /// Parses an [`Andd`] from the bytes.
    ///
    /// # Panics
    ///
    /// This method may panic if the bytes do not represent a valid [`Andd`].
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let header = AnddHeader::from_bytes(bytes);
        debug_assert_eq!(header.length as usize, bytes.len());

        let header_len = core::mem::size_of::<AnddHeader>();
        let acpi_object_name = core::str::from_utf8(&bytes[header_len..])
            .unwrap()
            .to_owned();

        Self {
            header,
            acpi_object_name,
        }
    }
}
