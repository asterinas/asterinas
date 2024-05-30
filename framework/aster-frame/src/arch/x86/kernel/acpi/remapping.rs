// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

//! Remapping structures of DMAR table.
//! This file defines these structures and provides a "Debug" implementation to see the value inside these structures.
//! Most of the introduction are copied from Intel vt-directed-io-specification.

use alloc::{string::String, vec::Vec};
use core::{fmt::Debug, mem::size_of};

/// DMA-remapping hardware unit definition (DRHD).
///
/// A DRHD structure uniquely represents a remapping hardware unit present in the platform.
/// There must be at least one instance of this structure for each
/// PCI segment in the platform.
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
pub struct AtsrHeader {
    typ: u16,
    length: u16,
    flags: u8,
    reserved: u8,
    segment_num: u16,
}

/// Remapping Hardware Status Affinity (RHSA).
///
/// It is applicable for platforms supporting non-uniform memory (NUMA), where Remapping hardware units spans across nodes.
/// This optional structure provides the association between each Remapping hardware unit (identified by its
/// espective Base Address) and the proximity domain to which that hardware unit belongs.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
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
///
#[derive(Debug, Clone)]
pub struct Sidp {
    header: SidpHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
pub struct DeviceScopeHeader {
    typ: u8,
    length: u8,
    flags: u8,
    reserved: u8,
    enum_id: u8,
    start_bus_number: u8,
}

macro_rules! impl_from_bytes {
    ($(($struct:tt,$header_struct:tt,$dst_name:ident)),*) => {
        $(impl $struct {
            /// Creates instance from bytes
            ///
            /// # Safety
            ///
            /// User must ensure the bytes is valid.
            ///
            pub unsafe fn from_bytes(bytes: &[u8]) -> Self {
                let length = u16_from_slice(&bytes[2..4]) as usize;
                debug_assert_eq!(length, bytes.len());

                let mut index = core::mem::size_of::<$header_struct>();
                let mut remain_length = length - core::mem::size_of::<$header_struct>();
                let mut $dst_name = Vec::new();
                while remain_length > 0 {
                    let length = *bytes[index + 1..index + 2].as_ptr() as usize;
                    let temp = DeviceScope::from_bytes(
                        &bytes[index..index + length],
                    );
                    $dst_name.push(temp);
                    index += length;
                    remain_length -= length;
                }

                let header = *(bytes.as_ptr() as *const $header_struct);
                Self{
                    header,
                    $dst_name
                }
            }
        })*
    };
}

impl_from_bytes!(
    (Drhd, DrhdHeader, device_scopes),
    (Rmrr, RmrrHeader, device_scopes),
    (Atsr, AtsrHeader, device_scopes),
    (Satc, SatcHeader, device_scopes),
    (Sidp, SidpHeader, device_scopes)
);

impl DeviceScope {
    /// Creates instance from bytes
    ///
    /// # Safety
    ///
    /// User must ensure the bytes is valid.
    ///
    unsafe fn from_bytes(bytes: &[u8]) -> Self {
        let length = bytes[1] as u32;
        debug_assert_eq!(length, bytes.len() as u32);
        let header = *(bytes.as_ptr() as *const DeviceScopeHeader);

        let mut index = size_of::<DeviceScopeHeader>();
        let mut remain_length = length - index as u32;
        let mut path = Vec::new();
        while remain_length > 0 {
            let temp: (u8, u8) = *(bytes[index..index + 2].as_ptr() as *const (u8, u8));
            path.push(temp);
            index += 2;
            remain_length -= 2;
        }

        Self { header, path }
    }
}

impl Rhsa {
    /// Creates instance from bytes
    ///
    /// # Safety
    ///
    /// User must ensure the bytes is valid.
    ///
    pub unsafe fn from_bytes(bytes: &[u8]) -> Self {
        let length = u16_from_slice(&bytes[2..4]) as u32;
        debug_assert_eq!(length, bytes.len() as u32);
        *(bytes.as_ptr() as *const Self)
    }
}

impl Andd {
    /// Creates instance from bytes
    ///
    /// # Safety
    ///
    /// User must ensure the bytes is valid.
    ///
    pub unsafe fn from_bytes(bytes: &[u8]) -> Self {
        let length = u16_from_slice(&bytes[2..4]) as usize;
        debug_assert_eq!(length, bytes.len());

        let index = core::mem::size_of::<AnddHeader>();
        let remain_length = length - core::mem::size_of::<AnddHeader>();
        let string = String::from_utf8(bytes[index..index + length].to_vec()).unwrap();

        let header = *(bytes.as_ptr() as *const AnddHeader);
        Self {
            header,
            acpi_object_name: string,
        }
    }
}

fn u16_from_slice(input: &[u8]) -> u16 {
    u16::from_ne_bytes(input[0..size_of::<u16>()].try_into().unwrap())
}
