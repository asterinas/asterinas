#![allow(non_camel_case_types)]
use crate::prelude::*;

pub type sigset_t = u64;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct sigaction_t {
    pub handler_ptr: Vaddr,
    pub flags: u32,
    pub restorer_ptr: Vaddr,
    pub mask: sigset_t,
}
