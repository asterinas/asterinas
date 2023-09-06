use crate::device::{Device, DeviceType};
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct Metadata {
    pub dev: u64,
    pub ino: usize,
    pub size: usize,
    pub blk_size: usize,
    pub blocks: usize,
    pub atime: Duration,
    pub mtime: Duration,
    pub ctime: Duration,
    pub type_: InodeType,
    pub mode: InodeMode,
    pub nlinks: usize,
    pub uid: usize,
    pub gid: usize,
    pub rdev: u64,
}

impl Metadata {
    pub fn new_dir(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            dev: 0,
            ino,
            size: 2,
            blk_size: sb.bsize,
            blocks: 1,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
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
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
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
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::SymLink,
            mode,
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }
    pub fn new_device(ino: usize, mode: InodeMode, sb: &SuperBlock, device: &dyn Device) -> Self {
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size: sb.bsize,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::from(device.type_()),
            mode,
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: device.id().into(),
        }
    }

    pub fn new_socket(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Metadata {
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size: sb.bsize,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::Socket,
            mode,
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SuperBlock {
    pub magic: u64,
    pub bsize: usize,
    pub blocks: usize,
    pub bfree: usize,
    pub bavail: usize,
    pub files: usize,
    pub ffree: usize,
    pub fsid: u64,
    pub namelen: usize,
    pub frsize: usize,
    pub flags: u64,
}

impl SuperBlock {
    pub fn new(magic: u64, block_size: usize, name_max_len: usize) -> Self {
        Self {
            magic,
            bsize: block_size,
            blocks: 0,
            bfree: 0,
            bavail: 0,
            files: 0,
            ffree: 0,
            fsid: 0,
            namelen: name_max_len,
            frsize: block_size,
            flags: 0,
        }
    }
}

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

impl InodeType {
    pub fn support_read(&self) -> bool {
        matches!(
            self,
            InodeType::File | InodeType::Socket | InodeType::CharDevice | InodeType::BlockDevice
        )
    }

    pub fn support_write(&self) -> bool {
        matches!(
            self,
            InodeType::File | InodeType::Socket | InodeType::CharDevice | InodeType::BlockDevice
        )
    }

    pub fn is_reguler_file(&self) -> bool {
        *self == InodeType::File
    }

    pub fn is_directory(&self) -> bool {
        *self == InodeType::Dir
    }
}

impl From<DeviceType> for InodeType {
    fn from(type_: DeviceType) -> InodeType {
        match type_ {
            DeviceType::CharDevice => InodeType::CharDevice,
            DeviceType::BlockDevice => InodeType::BlockDevice,
        }
    }
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
