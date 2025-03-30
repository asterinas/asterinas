// SPDX-License-Identifier: MPL-2.0

//! I/O port access.

use core::marker::PhantomData;

pub struct WriteOnlyAccess;
pub struct ReadWriteAccess;

pub trait IoPortWriteAccess {}
pub trait IoPortReadAccess {}

impl IoPortWriteAccess for WriteOnlyAccess {}
impl IoPortWriteAccess for ReadWriteAccess {}
impl IoPortReadAccess for ReadWriteAccess {}

pub trait PortRead: Sized {
    unsafe fn read_from_port(_port: u16) -> Self {
        unimplemented!()
    }
}

pub trait PortWrite: Sized {
    unsafe fn write_to_port(_port: u16, _value: Self) {
        unimplemented!()
    }
}

impl PortRead for u8 {}
impl PortWrite for u8 {}
impl PortRead for u16 {}
impl PortWrite for u16 {}
impl PortRead for u32 {}
impl PortWrite for u32 {}
