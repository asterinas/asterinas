// SPDX-License-Identifier: MPL-2.0

//! The util of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![feature(int_roundings)]

extern crate alloc;

pub mod coeff;
pub mod dup;
pub mod fixed_point;
pub mod mem_obj_slice;
pub mod per_cpu_counter;
pub mod printer;
pub mod ranged_integer;
pub mod safe_ptr;
pub mod slot_vec;
