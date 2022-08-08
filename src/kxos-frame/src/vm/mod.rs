//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

/// The size of a VM page or a page frame.
pub const PAGE_SIZE: usize = 0x1000; // 4KB

mod frame;
mod io;
mod pod;
mod space;

pub use self::frame::{VmAllocOptions, VmFrame, VmFrameVec, VmFrameVecIter};
pub use self::io::VmIo;
pub use self::pod::Pod;
pub use self::space::{VmMapOptions, VmPerm, VmSpace};
