// SPDX-License-Identifier: MPL-2.0

//! The util of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![feature(int_roundings)]

extern crate alloc;

pub mod coeff;
pub mod dup;
pub mod safe_ptr;
pub mod segment_slice;
pub mod slot_vec;
pub mod union_read_ptr;
