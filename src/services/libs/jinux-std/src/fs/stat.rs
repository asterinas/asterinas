#![allow(non_camel_case_types)]

type dev_t = usize;
type ino_t = usize;
type mode_t = u32;
type nlink_t = usize;
type uid_t = u32;
type gid_t = u32;
type off_t = u32;
type blksize_t = isize;
type blkcnt_t = isize;
type timespec = isize;

pub const S_IFMT: u32 = 0o170000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;

/// File Stat
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct Stat {
    /// ID of device containing file
    st_dev: dev_t,
    /// Inode number
    st_ino: ino_t,
    /// File type and mode
    st_mode: mode_t,
    /// Number of hard links
    st_nlink: nlink_t,
    /// User ID of owner
    st_uid: uid_t,
    /// Group ID of owner
    st_gid: gid_t,
    /// Device ID (if special file)
    st_rdev: dev_t,
    /// Total size, in bytes
    st_size: off_t,
    /// Block size for filesystem I/O
    st_blksize: blksize_t,
    /// Number of 512B blocks allocated
    st_blocks: blkcnt_t,
    /// Time of last access
    st_atime: timespec,
    /// Time of last modification
    st_mtime: timespec,
    /// Time of last status change
    st_ctime: timespec,
}

impl Stat {
    /// We use the same stat as linux
    pub fn stdout_stat() -> Self {
        let mut stat = Stat::default();
        stat.st_mode = S_IFCHR | 0o620;
        stat.st_nlink = 1;
        stat.st_blksize = 1024;
        stat
    }

    /// Fake stat for a dir
    pub fn fake_dir_stat() -> Self {
        let mut stat = Stat::default();
        stat.st_mode = S_IFDIR | 0o755;
        stat.st_nlink = 20;
        stat.st_blksize = 4096;
        stat
    }
}
