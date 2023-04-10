use crate::prelude::*;

// The definition of MMapFlags is from occlum
bitflags! {
    pub struct MMapFlags : u32 {
        const MAP_FILE            = 0x0;
        const MAP_SHARED          = 0x1;
        const MAP_PRIVATE         = 0x2;
        const MAP_SHARED_VALIDATE = 0x3;
        const MAP_TYPE            = 0xf;
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

impl TryFrom<u64> for MMapFlags {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        MMapFlags::from_bits(value as u32)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown mmap flags"))
    }
}
