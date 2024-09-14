// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

mod element;
mod inode;

use alloc::sync::Arc;

pub use element::{DataProvider, KernfsElem};
pub use inode::{KernfsNode, KernfsNodeFlag};

use super::utils::FileSystem;
use crate::prelude::*;

/// Block size.
const BLOCK_SIZE: usize = 1024;

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
/// - `alloc_id(&self) -> u64`: Allocates and returns a unique identifier for
///   inodes or other filesystem entities.
///
/// - `init(&self) -> Result<()>`: Initializes the pseudo filesystem, setting
///   up the root directory and any necessary internal structures.
///
/// - `fs(&self) -> Arc<dyn FileSystem>`: Returns a reference to the filesystem
///   itself, allowing for dynamic dispatch to the underlying filesystem
///   implementation.
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
///     fn alloc_id(&self) -> u64 {
///         // Allocate and return a unique ID
///     }
///
///     fn init(&self) -> Result<()> {
///         // Initialize the filesystem
///     }
///
///     fn fs(&self) -> Arc<dyn FileSystem> {
///         // Return a reference to the filesystem
///     }
/// }
/// ```
pub trait PseudoFileSystem: FileSystem {
    fn alloc_id(&self) -> u64;

    fn init(&self) -> Result<()>;

    fn fs(&self) -> Arc<dyn FileSystem>;
}

/// A toy pseudo filesystem that is working as sysfs.
/// It takes effect if you mount it to `/sys`.
#[cfg(ktest)]
mod tests {
    use core::sync::atomic::{AtomicU64, Ordering};

    use ostd::{mm::VmWriter, prelude::ktest};

    use crate::{
        fs::{
            kernfs::{DataProvider, KernfsNode, KernfsNodeFlag, PseudoFileSystem},
            utils::{FileSystem, FsFlags, Inode, SuperBlock, NAME_MAX},
        },
        prelude::*,
    };

    /// ToySysFS filesystem.
    /// Magic number.
    const SYSFS_MAGIC: u64 = 0x9fa0;
    /// Root Inode ID.
    const SYSFS_ROOT_INO: u64 = 1;
    /// Block size.
    const BLOCK_SIZE: usize = 1024;

    pub struct ToySysFS {
        sb: SuperBlock,
        root: Arc<dyn Inode>,
        inode_allocator: AtomicU64,
        this: Weak<Self>,
    }

    impl ToySysFS {
        pub fn new() -> Arc<Self> {
            Arc::new_cyclic(|weak_fs| Self {
                sb: SuperBlock::new(SYSFS_MAGIC, BLOCK_SIZE, NAME_MAX),
                root: SysfsRoot::new_inode(weak_fs.clone()),
                inode_allocator: AtomicU64::new(SYSFS_ROOT_INO + 1),
                this: weak_fs.clone(),
            })
        }
    }

    impl PseudoFileSystem for ToySysFS {
        fn alloc_id(&self) -> u64 {
            self.inode_allocator.fetch_add(1, Ordering::SeqCst)
        }

        fn init(&self) -> Result<()> {
            let root = self.root_inode();
            let root = root.downcast_ref::<KernfsNode>().unwrap();
            KernfsNode::new_dir("block", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("bus", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("class", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("dev", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("firmware", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("fs", None, KernfsNodeFlag::empty(), root.this_weak())?;
            let kernel =
                KernfsNode::new_dir("kernel", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("module", None, KernfsNodeFlag::empty(), root.this_weak())?;
            KernfsNode::new_dir("power", None, KernfsNodeFlag::empty(), root.this_weak())?;
            let ab = KernfsNode::new_attr(
                "address_bits",
                None,
                KernfsNodeFlag::empty(),
                kernel.this_weak(),
            )?;
            ab.set_data(Box::new(AddressBits)).unwrap();
            let cpu_byteorder = KernfsNode::new_attr(
                "byteorder",
                None,
                KernfsNodeFlag::empty(),
                kernel.this_weak(),
            )?;
            cpu_byteorder
                .set_data(Box::new(CpuByteOrder::new()))
                .unwrap();
            Ok(())
        }

        fn fs(&self) -> Arc<dyn FileSystem> {
            self.this.upgrade().unwrap()
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
            self.sb.clone()
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
            KernfsNode::new_root("sysfs", fs, SYSFS_ROOT_INO, BLOCK_SIZE)
        }
    }

    pub struct AddressBits;

    impl DataProvider for AddressBits {
        fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize> {
            let data = "64\n".as_bytes().to_vec();
            let start = data.len().min(offset);
            let end = data.len().min(offset + writer.avail());
            let len = end - start;
            writer.write_fallible(&mut (&data[start..end]).into())?;
            Ok(len)
        }

        fn write_at(&mut self, _reader: &mut VmReader, _offset: usize) -> Result<usize> {
            return_errno_with_message!(Errno::EINVAL, "cpuinfo is read-only");
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
        fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize> {
            let start = self.byte_order.len().min(offset);
            let end = self.byte_order.len().min(offset + writer.avail());
            let len = end - start;
            writer.write_fallible(&mut (&self.byte_order[start..end]).into())?;
            Ok(len)
        }

        fn write_at(&mut self, reader: &mut VmReader, offset: usize) -> Result<usize> {
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
    }

    fn load_fs() -> Arc<ToySysFS> {
        crate::time::clocks::init_for_ktest();
        let fs: Arc<ToySysFS> = ToySysFS::new();
        fs
    }

    #[ktest]
    fn new_fs() {
        let fs = load_fs();
        assert_eq!(fs.alloc_id(), SYSFS_ROOT_INO + 1);
    }

    #[ktest]
    fn init_fs() {
        let fs = load_fs();
        fs.init().unwrap();
    }

    #[ktest]
    fn read_address_bits() {
        let fs = load_fs();
        fs.init().unwrap();
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
        fs.init().unwrap();
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
        fs.init().unwrap();
        let root = fs.root_inode();
        let kernel = root.lookup("kernel").unwrap();
        let cpu_byteorder = kernel.lookup("byteorder").unwrap();
        let mut write_buf = "big\n".as_bytes().to_vec();
        let mut reader = VmReader::from(&write_buf as &[u8]).to_fallible();
        let write_len = cpu_byteorder.write_at(0, &mut reader).unwrap();
        assert_eq!(write_len, "big\n".as_bytes().len());
        let mut read_buf = vec![0u8; "big\n".as_bytes().len()];
        let mut writer = VmWriter::from(&mut read_buf as &mut [u8]).to_fallible();
        cpu_byteorder.read_at(0, &mut writer).unwrap();
        assert_eq!(read_buf, "big\n".as_bytes());
    }
}
