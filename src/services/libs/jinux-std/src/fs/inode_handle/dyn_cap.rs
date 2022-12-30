use crate::prelude::*;

use crate::rights::{Rights, TRights};

use super::*;

impl InodeHandle<Rights> {
    pub fn new(
        inode: Arc<dyn Inode>,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Self> {
        let inode_info = inode.metadata();
        if access_mode.is_readable() && !inode_info.mode.is_readable() {
            return_errno_with_message!(Errno::EACCES, "File is not readable");
        }
        if access_mode.is_writable() && !inode_info.mode.is_writable() {
            return_errno_with_message!(Errno::EACCES, "File is not writable");
        }
        if access_mode.is_writable() && inode_info.type_ == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "Directory cannot open to write");
        }
        let inner = Arc::new(InodeHandle_ {
            inode,
            offset: Mutex::new(0),
            access_mode,
            status_flags: Mutex::new(status_flags),
        });
        Ok(Self(inner, Rights::from(access_mode)))
    }

    pub fn to_static<R1: TRights>(self) -> Result<InodeHandle<R1>> {
        let rights = Rights::from_bits(R1::BITS).ok_or(Error::new(Errno::EBADF))?;
        if !self.1.contains(rights) {
            return_errno_with_message!(Errno::EBADF, "check rights failed");
        }
        Ok(InodeHandle(self.0, R1::new()))
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "File is not readable");
        }
        self.0.read(buf)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "File is not writable");
        }
        self.0.write(buf)
    }

    pub fn readdir(&self, writer: &mut dyn DirentWriter) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "File is not readable");
        }
        self.0.readdir(writer)
    }
}

impl Clone for InodeHandle<Rights> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1.clone())
    }
}
