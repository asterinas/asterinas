use crate::prelude::*;

impl From<ext2::error::Error> for Error {
    fn from(error: ext2::error::Error) -> Self {
        match error {
            ext2::error::Error::BadMagic => {
                Error::with_message(Errno::EINVAL, "bad ext2 magic number")
            }
            ext2::error::Error::BadRevision => {
                Error::with_message(Errno::EINVAL, "bad ext2 version")
            }
            ext2::error::Error::BadCreaterOS => {
                Error::with_message(Errno::EINVAL, "bad ext2 creater os")
            }
            ext2::error::Error::BadBitMap => Error::with_message(Errno::EINVAL, "bad ext2 bitmap"),
            ext2::error::Error::BadDirEntry => {
                Error::with_message(Errno::EINVAL, "bad ext2 dir entry")
            }
            ext2::error::Error::NotSupported => Error::new(Errno::ENOSYS),
            ext2::error::Error::IsDir => Error::new(Errno::EISDIR),
            ext2::error::Error::NotDir => Error::new(Errno::ENOTDIR),
            ext2::error::Error::NotFound => Error::new(Errno::ENOENT),
            ext2::error::Error::Exist => Error::new(Errno::EEXIST),
            ext2::error::Error::NotSameFs => Error::with_message(Errno::EXDEV, "not same fs"),
            ext2::error::Error::InvalidParam => Error::new(Errno::EINVAL),
            ext2::error::Error::NoSpace => Error::with_message(Errno::ENOSPC, "no space on device"),
            ext2::error::Error::DirRemoved => {
                Error::with_message(Errno::ENOENT, "dir has been removed")
            }
            ext2::error::Error::DirNotEmpty => Error::new(Errno::ENOTEMPTY),
            ext2::error::Error::Again => Error::new(Errno::EAGAIN),
            ext2::error::Error::SymLoop => Error::new(Errno::ELOOP),
            ext2::error::Error::Busy => Error::new(Errno::EBUSY),
            ext2::error::Error::IoError => Error::new(Errno::EIO),
            ext2::error::Error::WriteProtected => Error::new(Errno::EROFS),
            ext2::error::Error::NoPerm => Error::new(Errno::EPERM),
            ext2::error::Error::NameTooLong => Error::new(Errno::ENAMETOOLONG),
            ext2::error::Error::FileTooBig => Error::new(Errno::EFBIG),
            ext2::error::Error::OpNotSupported => Error::new(Errno::EOPNOTSUPP),
            ext2::error::Error::NoMemory => Error::new(Errno::ENOMEM),
        }
    }
}

mod fs;
mod inode;
