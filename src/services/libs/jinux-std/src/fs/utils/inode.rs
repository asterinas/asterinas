use alloc::string::String;
use alloc::sync::Arc;
use bitflags::bitflags;
use core::any::Any;
use jinux_frame::vm::VmFrame;

use super::{DirentWriterContext, FileSystem, IoctlCmd, SuperBlock};
use crate::prelude::*;

#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InodeType {
    NamedPipe = 0o010000,
    CharDevice = 0o020000,
    Dir = 0o040000,
    BlockDevice = 0o060000,
    File = 0o100000,
    SymLink = 0o120000,
    Socket = 0o140000,
}

bitflags! {
    pub struct InodeMode: u16 {
        /// set-user-ID
        const S_ISUID = 0o4000;
        /// set-group-ID
        const S_ISGID = 0o2000;
        /// sticky bit
        const S_ISVTX = 0o1000;
        /// read by owner
        const S_IRUSR = 0o0400;
        /// write by owner
        const S_IWUSR = 0o0200;
        /// execute/search by owner
        const S_IXUSR = 0o0100;
        /// read by group
        const S_IRGRP = 0o0040;
        /// write by group
        const S_IWGRP = 0o0020;
        /// execute/search by group
        const S_IXGRP = 0o0010;
        /// read by others
        const S_IROTH = 0o0004;
        /// write by others
        const S_IWOTH = 0o0002;
        /// execute/search by others
        const S_IXOTH = 0o0001;
    }
}

impl InodeMode {
    pub fn is_readable(&self) -> bool {
        self.contains(Self::S_IRUSR)
    }

    pub fn is_writable(&self) -> bool {
        self.contains(Self::S_IWUSR)
    }

    pub fn is_executable(&self) -> bool {
        self.contains(Self::S_IXUSR)
    }

    pub fn has_sticky_bit(&self) -> bool {
        self.contains(Self::S_ISVTX)
    }

    pub fn has_set_uid(&self) -> bool {
        self.contains(Self::S_ISUID)
    }

    pub fn has_set_gid(&self) -> bool {
        self.contains(Self::S_ISGID)
    }
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub dev: usize,
    pub ino: usize,
    pub size: usize,
    pub blk_size: usize,
    pub blocks: usize,
    pub atime: Timespec,
    pub mtime: Timespec,
    pub ctime: Timespec,
    pub type_: InodeType,
    pub mode: InodeMode,
    pub nlinks: usize,
    pub uid: usize,
    pub gid: usize,
    pub rdev: usize,
}

impl Metadata {
    pub fn new_dir(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            dev: 0,
            ino,
            size: 2,
            blk_size: sb.bsize,
            blocks: 0,
            atime: Timespec { sec: 0, nsec: 0 },
            mtime: Timespec { sec: 0, nsec: 0 },
            ctime: Timespec { sec: 0, nsec: 0 },
            type_: InodeType::Dir,
            mode,
            nlinks: 2,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }

    pub fn new_file(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size: sb.bsize,
            blocks: 0,
            atime: Timespec { sec: 0, nsec: 0 },
            mtime: Timespec { sec: 0, nsec: 0 },
            ctime: Timespec { sec: 0, nsec: 0 },
            type_: InodeType::File,
            mode,
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }

    pub fn new_symlink(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size: sb.bsize,
            blocks: 0,
            atime: Timespec { sec: 0, nsec: 0 },
            mtime: Timespec { sec: 0, nsec: 0 },
            ctime: Timespec { sec: 0, nsec: 0 },
            type_: InodeType::SymLink,
            mode,
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }
}

#[derive(Default, Copy, Clone, Pod, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(C)]
pub struct Timespec {
    pub sec: i64,
    pub nsec: i64,
}

pub trait Inode: Any + Sync + Send {
    fn len(&self) -> usize;

    fn resize(&self, new_size: usize);

    fn metadata(&self) -> Metadata;

    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()>;

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()>;

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize>;

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize>;

    fn mknod(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>>;

    fn readdir(&self, ctx: &mut DirentWriterContext) -> Result<usize>;

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()>;

    fn unlink(&self, name: &str) -> Result<()>;

    fn rmdir(&self, name: &str) -> Result<()>;

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>>;

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()>;

    fn read_link(&self) -> Result<String>;

    fn write_link(&self, target: &str) -> Result<()>;

    fn ioctl(&self, cmd: &IoctlCmd) -> Result<()>;

    fn sync(&self) -> Result<()>;

    fn fs(&self) -> Arc<dyn FileSystem>;

    fn as_any_ref(&self) -> &dyn Any;
}

impl dyn Inode {
    pub fn downcast_ref<T: Inode>(&self) -> Option<&T> {
        self.as_any_ref().downcast_ref::<T>()
    }
}
