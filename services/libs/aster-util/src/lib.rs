//! The util of Asterinas.
#![no_std]
#![forbid(unsafe_code)]
#![feature(int_roundings)]

extern crate alloc;

pub mod coeff;
pub mod dup;
pub mod id_allocator;
pub mod safe_ptr;
pub mod slot_vec;
pub mod union_read_ptr;
