//! The vfs components of jinux
#![no_std]
#![forbid(unsafe_code)]
#![allow(unused_variables)]
#![feature(btree_extract_if)]
#![feature(trait_upcasting)]

extern crate alloc;

pub mod device;
pub mod dirent_visitor;
pub mod direntry_vec;
pub mod events;
pub mod fs;
pub mod inode;
pub mod io_events;
pub mod ioctl;
pub mod metadata;
pub mod poll;

mod prelude;
