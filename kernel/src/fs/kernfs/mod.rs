// SPDX-License-Identifier: MPL-2.0

mod element;
mod inode;

pub use element::PseudoElement;
pub use inode::PseudoINode;

use super::utils::{FileSystem, Inode};
use crate::prelude::*;

/// Block size.
const BLOCK_SIZE: usize = 1024;

/// Provides interfaces for reading and writing to a pseudo file.
/// It is used by the pseudo file system to read and write data.
///
/// # Example
///
/// ```rust
/// use crate::fs::kernfs::DataProvider;
/// use crate::prelude::*;
///
/// struct MyDataProvider {
///    data: Vec<u8>,
/// }
///
/// impl MyDataProvider {
///   pub fn new(data: Vec<u8>) -> Self {
///      Self { data }
///  }
/// }
///
/// impl DataProvider for MyDataProvider {
///    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
///      let start = offset.min(self.data.len());
///      let end = (offset + writer.avail()).min(self.data.len());
///      let len = end - start;
///      writer.write_all(&self.data[start..end])?;
///      Ok(len)
///     }
///
///   fn write_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize> {
///      let write_len = reader.remain();
///      let end = offset + write_len;
///
///      if self.data.len() < end {
///         self.data.resize(end, 0);
///         }
///
///      let mut writer = VmWriter::from(&mut self.data[offset..end]);
///      let value = reader.read_fallible(&mut writer)?;
///      if value != write_len {
///         return_errno!(Errno::EINVAL);
///         }
///
///      Ok(write_len)
///     }
///
///  fn truncate(&mut self, new_size: usize) -> Result<()> {
///     self.data.resize(new_size, 0);
///     Ok(())
///    }
/// }
/// ```
pub trait DataProvider: Any + Sync + Send {
    /// Reads data from the file at the given offset.
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    /// Writes data to the file at the given offset.
    fn write_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize>;
    /// Truncates the file to the given size.
    fn truncate(&mut self, new_size: usize) -> Result<()>;
}

/// Provides different abstractions for different pseudo-file system requirements.
/// It processes events and dynamic logic.
///
/// # Example
///
/// ```rust
/// use crate::fs::{
///     kernfs::PseudoExt,
///     sysfs::Action,
/// };
///
///
/// struct MyPseudoExt {
///     subject: Subject<Action>,
/// }
///
/// impl PseudoExt for MyPseudoExt {
///    fn on_create(&self, name: String, node: Arc<dyn Inode>) -> Result<()> {
///       // Implementation details...
///       self.subject.notify_observers(Action::Add);
///   }
///
///    fn on_remove(&self, name: String) -> Result<()> {
///       // Implementation details...
///       self.subject.notify_observers(Action::Remove);
///   }
/// }
/// ```
pub trait PseudoExt: Send + Sync {
    /// Creates a new node.
    fn on_create(&self, name: &str, node: Arc<dyn Inode>) -> Result<()>;
    /// Removes a node.
    fn on_remove(&self, name: &str) -> Result<()>;
}

/// A trait for the pseudo filesystem.
///
/// The pseudo filesystem is a virtual filesystem that is used to provide
/// a consistent interface for the kernel to interact with the underlying
/// hardware. It is typically mounted at a specific mount point (e.g., `/sys`)
/// and provides a hierarchical view of system resources and configurations.
///
/// This trait defines the basic operations that a pseudo filesystem must
/// implement, including allocating unique identifiers, initializing the
/// filesystem, and providing access to the filesystem itself.
///
/// # Methods
///
/// - `alloc_unique_id(&self) -> u64`: Allocates and returns a unique identifier for
///   inodes or other filesystem entities.
///
/// # Example
///
/// ```rust
/// use crate::fs::PseudoFileSystem;
///
/// struct MyPseudoFS {
///     // Implementation details...
/// }
///
/// impl PseudoFileSystem for MyPseudoFS {
///     fn alloc_unique_id(&self) -> u64 {
///         // Allocate and return a unique ID
///     }
/// }
/// ```
pub trait PseudoFileSystem: FileSystem {
    /// Allocates a unique ID for the inode.
    fn alloc_unique_id(&self) -> u64;
}

/// A toy pseudo filesystem that is working as sysfs.
/// It takes effect if you mount it to `/sys`.
#[cfg(ktest)]
mod tests {
    use core::sync::atomic::{AtomicU64, Ordering};

    use ostd::{mm::VmWriter, prelude::ktest};

    use crate::{
        fs::{
            kernfs::{DataProvider, PseudoFileSystem, PseudoINode},
            utils::{FileSystem, FsFlags, Inode, SuperBlock},
        },
        prelude::*,
    };

    /// ToySysFS filesystem.
    /// Root Inode ID.
    const SYSFS_ROOT_INO: u64 = 1;
    /// Block size.
    const BLOCK_SIZE: usize = 1024;

    pub struct ToySysFS {
        root: Arc<dyn Inode>,
        inode_allocator: AtomicU64,
    }

    impl ToySysFS {
        pub fn new() -> Arc<Self> {
            let fs = Arc::new_cyclic(|weak_fs| Self {
                root: SysfsRoot::new_inode(weak_fs.clone()),
                inode_allocator: AtomicU64::new(SYSFS_ROOT_INO + 1),
            });
            let root = fs.root_inode();
            let root = root.downcast_ref::<PseudoINode>().unwrap();
            let kernel = PseudoINode::new_dir("kernel", None, None, root.this_weak()).unwrap();
            let _ = PseudoINode::new_attr(
                "address_bits",
                None,
                Some(Box::new(AddressBits)),
                None,
                kernel.this_weak(),
            )
            .unwrap();
            let _ = PseudoINode::new_attr(
                "byteorder",
                None,
                Some(Box::new(CpuByteOrder::new())),
                None,
                kernel.this_weak(),
            )
            .unwrap();
            fs
        }
    }

    impl PseudoFileSystem for ToySysFS {
        /// Allocates a unique ID for the inode.
        fn alloc_unique_id(&self) -> u64 {
            self.inode_allocator.fetch_add(1, Ordering::SeqCst)
        }
    }

    impl FileSystem for ToySysFS {
        fn sync(&self) -> Result<()> {
            Ok(())
        }

        fn root_inode(&self) -> Arc<dyn Inode> {
            self.root.clone()
        }

        fn sb(&self) -> SuperBlock {
            unimplemented!()
        }

        fn flags(&self) -> FsFlags {
            FsFlags::empty()
        }
    }

    /// Represents the inode at `/sys`.
    /// Root directory of the sysfs.
    pub struct SysfsRoot;

    impl SysfsRoot {
        pub fn new_inode(fs: Weak<ToySysFS>) -> Arc<dyn Inode> {
            PseudoINode::new_root(fs, SYSFS_ROOT_INO, BLOCK_SIZE, None)
        }
    }

    pub struct AddressBits;

    impl DataProvider for AddressBits {
        fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
            let data = "64\n".as_bytes().to_vec();
            let start = data.len().min(offset);
            let end = data.len().min(offset + writer.avail());
            let len = end - start;
            writer.write_fallible(&mut (&data[start..end]).into())?;
            Ok(len)
        }

        fn write_at(&mut self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
            return_errno_with_message!(Errno::EINVAL, "cpuinfo is read-only");
        }

        fn truncate(&mut self, _new_size: usize) -> Result<()> {
            Ok(())
        }
    }

    pub struct CpuByteOrder {
        byte_order: Vec<u8>,
    }

    impl CpuByteOrder {
        pub fn new() -> Self {
            let byte_order = if cfg!(target_endian = "little") {
                "little\n".as_bytes().to_vec()
            } else {
                "big\n".as_bytes().to_vec()
            };
            Self { byte_order }
        }
    }

    impl Default for CpuByteOrder {
        fn default() -> Self {
            Self::new()
        }
    }

    impl DataProvider for CpuByteOrder {
        fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
            let start = self.byte_order.len().min(offset);
            let end = self.byte_order.len().min(offset + writer.avail());
            let len = end - start;
            writer.write_fallible(&mut (&self.byte_order[start..end]).into())?;
            Ok(len)
        }

        fn write_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize> {
            let write_len = reader.remain();
            let end = offset + write_len;

            if self.byte_order.len() < end {
                self.byte_order.resize(end, 0);
            }

            let mut writer = VmWriter::from(&mut self.byte_order[offset..end]);
            let value = reader.read_fallible(&mut writer)?;
            if value != write_len {
                return_errno!(Errno::EINVAL);
            }

            Ok(write_len)
        }

        fn truncate(&mut self, new_size: usize) -> Result<()> {
            self.byte_order.resize(new_size, 0);
            Ok(())
        }
    }

    fn load_fs() -> Arc<ToySysFS> {
        crate::time::clocks::init_for_ktest();
        let fs: Arc<ToySysFS> = ToySysFS::new();
        fs
    }

    #[ktest]
    fn new_fs() {
        let fs = load_fs();
        assert_eq!(fs.alloc_unique_id(), SYSFS_ROOT_INO + 4);
    }

    #[ktest]
    fn read_address_bits() {
        let fs = load_fs();
        let root = fs.root_inode();
        let kernel = root.lookup("kernel").unwrap();
        let ab = kernel.lookup("address_bits").unwrap();
        let mut read_buf = vec![0u8; "64\n".as_bytes().len()];
        let mut writer = VmWriter::from(&mut read_buf as &mut [u8]).to_fallible();
        ab.read_at(0, &mut writer).unwrap();
        assert_eq!(read_buf, "64\n".as_bytes());
    }

    #[ktest]
    fn read_cpu_byteorder() {
        let fs = load_fs();
        let root = fs.root_inode();
        let kernel = root.lookup("kernel").unwrap();
        let cpu_byteorder = kernel.lookup("byteorder").unwrap();
        let mut read_buf = vec![0u8; "little\n".as_bytes().len()];
        let mut writer = VmWriter::from(&mut read_buf as &mut [u8]).to_fallible();
        cpu_byteorder.read_at(0, &mut writer).unwrap();
        assert_eq!(read_buf, "little\n".as_bytes());
    }

    #[ktest]
    fn write_cpu_byteorder() {
        let fs = load_fs();
        let root = fs.root_inode();
        let kernel = root.lookup("kernel").unwrap();
        let cpu_byteorder = kernel.lookup("byteorder").unwrap();
        let write_buf = "big\n".as_bytes().to_vec();
        let mut reader = VmReader::from(&write_buf as &[u8]).to_fallible();
        let write_len = cpu_byteorder.write_at(0, &mut reader).unwrap();
        assert_eq!(write_len, "big\n".as_bytes().len());
        let mut read_buf = vec![0u8; "big\n".as_bytes().len()];
        let mut writer = VmWriter::from(&mut read_buf as &mut [u8]).to_fallible();
        cpu_byteorder.read_at(0, &mut writer).unwrap();
        assert_eq!(read_buf, "big\n".as_bytes());
    }
}
