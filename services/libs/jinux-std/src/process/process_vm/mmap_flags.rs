//! Definition of MMap flags, conforming to the linux mmap interface:
//! https://man7.org/linux/man-pages/man2/mmap.2.html
//!
//! The first 4 bits of the flag value represents the type of memory map,
//! while other bits are used as memory map flags.
//!

use crate::prelude::*;

// The map type mask
const MAP_TYPE: u32 = 0xf;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum MMapType {
    MapFile = 0x0,
    MapShared = 0x1,
    MapPrivate = 0x2,
    MapSharedValidate = 0x3,
}

bitflags! {
    pub struct MMapFlags : u32 {
        const MAP_FIXED           = 0x10;
        const MAP_ANONYMOUS       = 0x20;
        const MAP_GROWSDOWN       = 0x100;
        const MAP_DENYWRITE       = 0x800;
        const MAP_EXECUTABLE      = 0x1000;
        const MAP_LOCKED          = 0x2000;
        const MAP_NORESERVE       = 0x4000;
        const MAP_POPULATE        = 0x8000;
        const MAP_NONBLOCK        = 0x10000;
        const MAP_STACK           = 0x20000;
        const MAP_HUGETLB         = 0x40000;
        const MAP_SYNC            = 0x80000;
        const MAP_FIXED_NOREPLACE = 0x100000;
    }
}

#[derive(Debug)]
pub struct MMapOption {
    typ: MMapType,
    flags: MMapFlags,
}

impl TryFrom<u64> for MMapOption {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        let typ_raw = value as u32 & MAP_TYPE;
        let flags_raw = value as u32 & !MAP_TYPE;
        let typ = match typ_raw {
            0x0 => MMapType::MapFile,
            0x1 => MMapType::MapShared,
            0x2 => MMapType::MapPrivate,
            0x3 => MMapType::MapSharedValidate,
            _ => return Err(Error::with_message(Errno::EINVAL, "unknown mmap flags")),
        };
        if let Some(flags) = MMapFlags::from_bits(flags_raw) {
            Ok(MMapOption {
                typ: typ,
                flags: flags,
            })
        } else {
            Err(Error::with_message(Errno::EINVAL, "unknown mmap flags"))
        }
    }
}

impl MMapOption {
    pub fn typ(&self) -> MMapType {
        self.typ
    }

    pub fn flags(&self) -> MMapFlags {
        self.flags
    }
}
