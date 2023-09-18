//! The util of jinux
#![no_std]
#![forbid(unsafe_code)]
#![feature(int_roundings)]

extern crate alloc;

pub mod bitmap;
pub mod dup;
pub mod safe_ptr;
pub mod slot_vec;
pub mod union_read_ptr;
