use crate::events::IoEvents;
use crate::prelude::*;
use crate::process::signal::Poller;
use aster_rights::{Rights, TRights};

use super::*;

impl InodeHandle<Rights> {
    pub fn new(
        dentry: Arc<Dentry>,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Self> {
        let inode = dentry.inode();
        if access_mode.is_readable() && !inode.mode().is_readable() {
            return_errno_with_message!(Errno::EACCES, "File is not readable");
        }
        if access_mode.is_writable() && !inode.mode().is_writable() {
            return_errno_with_message!(Errno::EACCES, "File is not writable");
        }
        if access_mode.is_writable() && inode.type_() == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "Directory cannot open to write");
        }

        let file_io = if let Some(device) = inode.as_device() {
            device.open()?
        } else {
            None
        };

        let inner = Arc::new(InodeHandle_ {
            dentry,
            file_io,
            offset: Mutex::new(0),
            access_mode,
            status_flags: AtomicU32::new(status_flags.bits()),
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

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "File is not readable");
        }

        self.0.read_to_end(buf)
    }

    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "File is not readable");
        }
        self.0.readdir(visitor)
    }
}

impl Clone for InodeHandle<Rights> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}

impl FileLike for InodeHandle<Rights> {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "File is not readable");
        }
        self.0.read(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "File is not writable");
        }
        self.0.write(buf)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.0.poll(mask, poller)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.0.ioctl(cmd, arg)
    }

    fn metadata(&self) -> Metadata {
        self.dentry().inode_metadata()
    }

    fn status_flags(&self) -> StatusFlags {
        self.0.status_flags()
    }

    fn set_status_flags(&self, new_status_flags: StatusFlags) -> Result<()> {
        self.0.set_status_flags(new_status_flags);
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        self.0.access_mode()
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        self.0.seek(seek_from)
    }

    fn clean_for_close(&self) -> Result<()> {
        // Close does not guarantee that the data has been successfully saved to disk.
        Ok(())
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        self.dentry().inode().as_device()
    }
}
