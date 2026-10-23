// SPDX-License-Identifier: GPL-1.0+
// Translated board portion: Linux drivers/net/ethernet/8390/ne2k-pci.c.
// A Linux device driver for PCI NE2000 clones.
// 1992-2000 by Donald Becker, NE2000 core and various modifications.
// 1995-1998 by Paul Gortmaker, core modifications and PCI support.
// Copyright 1993 assigned to the United States Government as represented
// by the Director, National Security Agency.
// This code is distributed under the GNU General Public License and must
// retain the authorship, copyright, and license notice.
// The source notice states that this file is not a complete program and may
// only be used when the entire operating system is licensed under the GPL.
// Donald Becker: becker@scyld.com, C/O Scyld Computing Corporation,
// 410 Severn Ave., Suite 210, Annapolis MD 21403.
//
// Translated shared-core portion: Linux drivers/net/ethernet/8390/lib8390.c.
// 8390.c: A general NS8390 ethernet driver core for Linux.
// Written 1992-94 by Donald Becker.
// Copyright 1993 United States Government as represented by the Director,
// National Security Agency.
// This chip-specific code must be combined with board-specific code.
// Paul Gortmaker: core modifications and cleanup.

//! Realtek RTL8029 PCI NE2000 support (x86-64 only).
//!
//! The PCI driver owns the ports and INTx mapping. Hard interrupts only mask
//! the device and schedule a taskless job; receive allocation and completion
//! processing run in the bottom half. The current network integration exposes
//! one device as `ne2k0` and rejects additional matching devices.
//!
//! This component is derived from GPL-licensed Linux code; see the notice above.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
#[cfg(target_arch = "x86_64")]
macro_rules! __log_prefix {
    () => {
        "ne2k: "
    };
}

#[cfg(target_arch = "x86_64")]
mod device;
#[cfg(target_arch = "x86_64")]
mod registers;

#[cfg(target_arch = "x86_64")]
pub use device::Ne2k;

/// The name under which the PCI network device is registered.
pub const DEVICE_NAME: &str = "ne2k0";

#[component::init_component]
fn init() -> Result<(), component::ComponentInitError> {
    #[cfg(target_arch = "x86_64")]
    device::register();
    Ok(())
}
