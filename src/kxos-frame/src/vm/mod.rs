//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

mod frame;
mod io;
mod pod;
mod space;

pub use self::frame::{VmAllocOptions, VmFrame, VmFrameVec, VmFrameVecIter};
pub use self::io::VmIo;
pub use self::pod::Pod;
pub use self::space::{VmMapOptions, VmPerm, VmSpace};
