use crate::prelude::*;

impl From<super::error::Error> for Error {
    fn from(error: super::error::Error) -> Self {
        match error {
            super::error::Error::BadMagic => {
                Error::with_message(Errno::EINVAL, "bad ext2 magic number")
            }
            super::error::Error::BadRevision => {
                Error::with_message(Errno::EINVAL, "bad ext2 version")
            }
            super::error::Error::BadCreaterOS => {
                Error::with_message(Errno::EINVAL, "bad ext2 creater os")
            }
            super::error::Error::BadBitMap => Error::with_message(Errno::EINVAL, "bad ext2 bitmap"),
            super::error::Error::BadDirEntry => {
                Error::with_message(Errno::EINVAL, "bad ext2 dir entry")
            }
            super::error::Error::NotSupported => Error::new(Errno::ENOSYS),
            super::error::Error::IsDir => Error::new(Errno::EISDIR),
            super::error::Error::NotDir => Error::new(Errno::ENOTDIR),
            super::error::Error::NotFound => Error::new(Errno::ENOENT),
            super::error::Error::Exist => Error::new(Errno::EEXIST),
            super::error::Error::NotSameFs => Error::with_message(Errno::EXDEV, "not same fs"),
            super::error::Error::InvalidParam => Error::new(Errno::EINVAL),
            super::error::Error::NoSpace => {
                Error::with_message(Errno::ENOSPC, "no space on device")
            }
            super::error::Error::DirRemoved => {
                Error::with_message(Errno::ENOENT, "dir has been removed")
            }
            super::error::Error::DirNotEmpty => Error::new(Errno::ENOTEMPTY),
            super::error::Error::Again => Error::new(Errno::EAGAIN),
            super::error::Error::SymLoop => Error::new(Errno::ELOOP),
            super::error::Error::Busy => Error::new(Errno::EBUSY),
            super::error::Error::IoError => Error::new(Errno::EIO),
            super::error::Error::WriteProtected => Error::new(Errno::EROFS),
            super::error::Error::NoPerm => Error::new(Errno::EPERM),
            super::error::Error::NameTooLong => Error::new(Errno::ENAMETOOLONG),
            super::error::Error::FileTooBig => Error::new(Errno::EFBIG),
            super::error::Error::OpNotSupported => Error::new(Errno::EOPNOTSUPP),
            super::error::Error::NoMemory => Error::new(Errno::ENOMEM),
        }
    }
}

mod fs;
mod inode;
