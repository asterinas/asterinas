// SPDX-License-Identifier: MPL-2.0

//! _bigtcp_ is a crate that wraps [`smoltcp`].
//!
//! [`smoltcp`] is designed for embedded systems where the number of sockets is always small. It
//! turns out that such a design cannot satisfy the need to implement the network stack of a
//! general-purpose OS kernel, in terms of flexibility and efficiency.
//!
//! The short-term goal of _bigtcp_ is to reuse the powerful TCP implementation of _smoltcp_, while
//! reimplementing Ethernet and IP protocols to increase the flexibility and performance of packet
//! dispatching.

#![no_std]
#![deny(unsafe_code)]
#![feature(btree_extract_if)]

pub mod device;
pub mod errors;
pub mod iface;
pub mod socket;
pub mod time;
pub mod wire;

extern crate alloc;
