// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

// Reference: <https://gitlab.com/virtio-fs/virtiofsd/-/blob/main/src/fuse.rs>

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct InHeader {
    pub len: u32,
    pub opcode: u32,
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub total_extlen: u16, // length of extensions in 8-byte units
    pub padding: u16,
}

impl InHeader {
    pub const fn new(len: u32, opcode: u32, unique: u64, nodeid: u64) -> Self {
        Self {
            len,
            opcode,
            unique,
            nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct OutHeader {
    pub len: u32,
    pub error: i32,
    pub unique: u64,
}

impl OutHeader {
    pub const fn new(len: u32, error: i32, unique: u64) -> Self {
        Self { len, error, unique }
    }

    pub const fn empty() -> Self {
        Self::new(0, 0, 0)
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct InitIn {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
    // The following fields are extensions.
    pub flags2: u32,
    pub unused: [u32; 11],
}

impl InitIn {
    pub const fn new(major: u32, minor: u32, max_readahead: u32, flags: u32, flags2: u32) -> Self {
        Self {
            major,
            minor,
            max_readahead,
            flags,
            flags2,
            unused: [0; 11],
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct InitOut {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
    pub max_background: u16,
    pub congestion_threshold: u16,
    pub max_write: u32,
    pub time_gran: u32,
    pub max_pages: u16,
    pub map_alignment: u16,
    pub flags2: u32,
    pub unused: [u32; 7],
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct OpenIn {
    pub flags: u32,
    pub open_flags: u32,
}

impl OpenIn {
    pub const fn new(flags: u32) -> Self {
        Self {
            flags,
            open_flags: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct ReleaseIn {
    pub fh: u64,
    pub flags: u32,
    pub release_flags: u32,
    pub lock_owner: u64,
}

impl ReleaseIn {
    pub const fn new(fh: u64, flags: u32) -> Self {
        Self {
            fh,
            flags,
            release_flags: 0,
            lock_owner: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct LseekIn {
    pub fh: u64,
    pub offset: i64,
    pub whence: u32,
    pub padding: u32,
}

impl LseekIn {
    pub const fn new(fh: u64, offset: i64, whence: u32) -> Self {
        Self {
            fh,
            offset,
            whence,
            padding: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct LseekOut {
    pub offset: i64,
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct OpenOut {
    pub fh: u64,
    pub open_flags: u32,
    pub padding: u32,
}

pub const FOPEN_DIRECT_IO: u32 = 1 << 0;
pub const FOPEN_KEEP_CACHE: u32 = 1 << 1;

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct GetattrIn {
    pub getattr_flags: u32,
    pub dummy: u32,
    pub fh: u64,
}

pub const FATTR_MODE: u32 = 1 << 0;
pub const FATTR_UID: u32 = 1 << 1;
pub const FATTR_GID: u32 = 1 << 2;
pub const FATTR_SIZE: u32 = 1 << 3;
pub const FATTR_ATIME: u32 = 1 << 4;
pub const FATTR_MTIME: u32 = 1 << 5;
pub const FATTR_FH: u32 = 1 << 6;
pub const FATTR_ATIME_NOW: u32 = 1 << 7;
pub const FATTR_MTIME_NOW: u32 = 1 << 8;
pub const FATTR_LOCKOWNER: u32 = 1 << 9;
pub const FATTR_CTIME: u32 = 1 << 10;

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy, Default)]
pub struct SetattrIn {
    pub valid: u32,
    pub padding: u32,
    pub fh: u64,
    pub size: u64,
    pub lock_owner: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atimensec: u32,
    pub mtimensec: u32,
    pub ctimensec: u32,
    pub mode: u32,
    pub unused4: u32,
    pub uid: u32,
    pub gid: u32,
    pub unused5: u32,
}

impl GetattrIn {
    pub const fn new(fh: u64) -> Self {
        Self {
            getattr_flags: 0,
            dummy: 0,
            fh,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct FuseAttrOut {
    pub attr_valid: u64,
    pub attr_valid_nsec: u32,
    pub dummy: u32,
    pub attr: Attr,
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct ReadIn {
    pub fh: u64,
    pub offset: u64,
    pub size: u32,
    pub read_flags: u32,
    pub lock_owner: u64,
    pub flags: u32,
    pub padding: u32,
}

impl ReadIn {
    pub const fn new(fh: u64, offset: u64, size: u32) -> Self {
        Self {
            fh,
            offset,
            size,
            read_flags: 0,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct WriteIn {
    pub fh: u64,
    pub offset: u64,
    pub size: u32,
    pub write_flags: u32,
    pub lock_owner: u64,
    pub flags: u32,
    pub padding: u32,
}

impl WriteIn {
    pub const fn new(fh: u64, offset: u64, size: u32) -> Self {
        Self {
            fh,
            offset,
            size,
            write_flags: 0,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct WriteOut {
    pub size: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct CreateIn {
    pub flags: u32,
    pub mode: u32,
    pub umask: u32,
    pub open_flags: u32,
}

impl CreateIn {
    pub const fn new(flags: u32, mode: u32) -> Self {
        Self {
            flags,
            mode,
            umask: 0,
            open_flags: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct MkdirIn {
    pub mode: u32,
    pub umask: u32,
}

impl MkdirIn {
    pub const fn new(mode: u32) -> Self {
        Self { mode, umask: 0 }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct MknodIn {
    pub mode: u32,
    pub rdev: u32,
    pub umask: u32,
    pub padding: u32,
}

impl MknodIn {
    pub const fn new(mode: u32, rdev: u32) -> Self {
        Self {
            mode,
            rdev,
            umask: 0,
            padding: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct ForgetIn {
    pub nlookup: u64,
}

impl ForgetIn {
    pub const fn new(nlookup: u64) -> Self {
        Self { nlookup }
    }
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct Attr {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atimensec: u32,
    pub mtimensec: u32,
    pub ctimensec: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct EntryOut {
    pub nodeid: u64,
    pub generation: u64,
    pub entry_valid: u64,
    pub attr_valid: u64,
    pub entry_valid_nsec: u32,
    pub attr_valid_nsec: u32,
    pub attr: Attr,
}

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct Dirent {
    pub ino: u64,
    pub off: u64,
    pub namelen: u32,
    pub typ: u32,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
pub enum FuseOpcode {
    Lookup = 1,
    Forget = 2,
    Getattr = 3,
    Setattr = 4,
    Readlink = 5,
    Symlink = 6,
    Mknod = 8,
    Mkdir = 9,
    Unlink = 10,
    Rmdir = 11,
    Rename = 12,
    Link = 13,
    Open = 14,
    Read = 15,
    Write = 16,
    Statfs = 17,
    Release = 18,
    Fsync = 20,
    Setxattr = 21,
    Getxattr = 22,
    Listxattr = 23,
    Removexattr = 24,
    Flush = 25,
    Init = 26,
    Opendir = 27,
    Readdir = 28,
    Releasedir = 29,
    Fsyncdir = 30,
    Getlk = 31,
    Setlk = 32,
    Setlkw = 33,
    Access = 34,
    Create = 35,
    Interrupt = 36,
    Bmap = 37,
    Destroy = 38,
    Ioctl = 39,
    Poll = 40,
    NotifyReply = 41,
    BatchForget = 42,
    Fallocate = 43,
    Readdirplus = 44,
    Rename2 = 45,
    Lseek = 46,
    CopyFileRange = 47,
    SetupMapping = 48,
    RemoveMapping = 49,
    SyncFs = 50,
    Tmpfile = 51,
}

impl From<FuseOpcode> for u32 {
    fn from(opcode: FuseOpcode) -> u32 {
        opcode as u32
    }
}

pub const FUSE_ROOT_ID: u64 = 1;

#[repr(C)]
#[derive(Debug, Pod, Clone, Copy)]
pub struct LinkIn {
    pub oldnodeid: u64,
}

impl LinkIn {
    pub const fn new(oldnodeid: u64) -> Self {
        Self { oldnodeid }
    }
}

pub const FUSE_KERNEL_VERSION: u32 = 7;
pub const FUSE_KERNEL_MINOR_VERSION: u32 = 38;

pub mod init_flags {
    pub const ASYNC_READ: u32 = 1 << 0;
    pub const POSIX_LOCKS: u32 = 1 << 1;
    pub const FILE_OPS: u32 = 1 << 2;
    pub const ATOMIC_O_TRUNC: u32 = 1 << 3;
    pub const EXPORT_SUPPORT: u32 = 1 << 4;
    pub const BIG_WRITES: u32 = 1 << 5;
    pub const DONT_MASK: u32 = 1 << 6;
    pub const SPLICE_WRITE: u32 = 1 << 7;
    pub const SPLICE_MOVE: u32 = 1 << 8;
    pub const SPLICE_READ: u32 = 1 << 9;
    pub const FLOCK_LOCKS: u32 = 1 << 10;
    pub const HAS_IOCTL_DIR: u32 = 1 << 11;
    pub const AUTO_INVAL_DATA: u32 = 1 << 12;
    pub const DO_READDIRPLUS: u32 = 1 << 13;
    pub const READDIRPLUS_AUTO: u32 = 1 << 14;
    pub const ASYNC_DIO: u32 = 1 << 15;
    pub const WRITEBACK_CACHE: u32 = 1 << 16;
    pub const NO_OPEN_SUPPORT: u32 = 1 << 17;
    pub const PARALLEL_DIROPS: u32 = 1 << 18;
    pub const HANDLE_KILLPRIV: u32 = 1 << 19;
    pub const POSIX_ACL: u32 = 1 << 20;
    pub const ABORT_ERROR: u32 = 1 << 21;
    pub const MAX_PAGES: u32 = 1 << 22;
    pub const CACHE_SYMLINKS: u32 = 1 << 23;
    pub const NO_OPENDIR_SUPPORT: u32 = 1 << 24;
    pub const EXPLICIT_INVAL_DATA: u32 = 1 << 25;
    pub const MAP_ALIGNMENT: u32 = 1 << 26;
    pub const SUBMOUNTS: u32 = 1 << 27;
    pub const HANDLE_KILLPRIV_V2: u32 = 1 << 28;
    pub const SETXATTR_EXT: u32 = 1 << 29;
    pub const INIT_EXT: u32 = 1 << 30;
    pub const SECURITY_CTX: u32 = 1 << 31;
}
