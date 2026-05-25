// SPDX-License-Identifier: MPL-2.0

//! I/O port abstractions.
//!
//! ARM64 platforms do not have port I/O. These stubs provide
//! the interface required by the generic I/O subsystem.

/// Trait for reading from an I/O port.
pub trait PortRead: Sized {}

/// Trait for writing to an I/O port.
pub trait PortWrite: Sized {}

impl PortRead for u8 {}
impl PortWrite for u8 {}
impl PortRead for u16 {}
impl PortWrite for u16 {}
impl PortRead for u32 {}
impl PortWrite for u32 {}
