use crate::events::{IoEvents, Observer};
use crate::prelude::*;
use crate::process::signal::Poller;

use super::file_handle::FileLike;
use super::utils::{AccessMode, Consumer, InodeMode, InodeType, Metadata, Producer, StatusFlags};

pub struct PipeReader {
    consumer: Consumer<u8>,
}

impl PipeReader {
    pub fn new(consumer: Consumer<u8>) -> Self {
        Self { consumer }
    }
}

impl FileLike for PipeReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.consumer.read(buf)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.consumer.poll(mask, poller)
    }

    fn status_flags(&self) -> StatusFlags {
        self.consumer.status_flags()
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.consumer.set_status_flags(new_flags)
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDONLY
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::NamedPipe,
            mode: InodeMode::from_bits_truncate(0o400),
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.consumer.register_observer(observer, mask)
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        self.consumer.unregister_observer(observer)
    }
}

pub struct PipeWriter {
    producer: Producer<u8>,
}

impl PipeWriter {
    pub fn new(producer: Producer<u8>) -> Self {
        Self { producer }
    }
}

impl FileLike for PipeWriter {
    fn write(&self, buf: &[u8]) -> Result<usize> {
        self.producer.write(buf)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.producer.poll(mask, poller)
    }

    fn status_flags(&self) -> StatusFlags {
        self.producer.status_flags()
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.producer.set_status_flags(new_flags)
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_WRONLY
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::NamedPipe,
            mode: InodeMode::from_bits_truncate(0o200),
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
        }
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.producer.register_observer(observer, mask)
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        self.producer.unregister_observer(observer)
    }
}

fn should_io_return(res: &Result<usize>, is_nonblocking: bool) -> bool {
    if is_nonblocking {
        return true;
    }
    match res {
        Ok(_) => true,
        Err(e) if e.error() == Errno::EAGAIN => false,
        Err(_) => true,
    }
}
