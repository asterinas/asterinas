// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::{
    file_handle::FileLike,
    utils::{AccessMode, Consumer, InodeMode, InodeType, Metadata, Producer, StatusFlags},
};
use crate::{
    events::{IoEvents, Observer},
    prelude::*,
    process::{
        signal::{Pollable, Poller},
        Gid, Uid,
    },
    time::clocks::RealTimeCoarseClock,
};

pub struct PipeReader {
    consumer: Consumer<u8>,
}

impl PipeReader {
    pub fn new(consumer: Consumer<u8>) -> Self {
        Self { consumer }
    }
}

impl Pollable for PipeReader {
    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.consumer.poll(mask, poller)
    }
}

impl FileLike for PipeReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.consumer.read(buf)
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
        let now = RealTimeCoarseClock::get().read_time();
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::NamedPipe,
            mode: InodeMode::from_bits_truncate(0o400),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
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
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
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

impl Pollable for PipeWriter {
    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.producer.poll(mask, poller)
    }
}

impl FileLike for PipeWriter {
    fn write(&self, buf: &[u8]) -> Result<usize> {
        self.producer.write(buf)
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
        let now = RealTimeCoarseClock::get().read_time();
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::NamedPipe,
            mode: InodeMode::from_bits_truncate(0o200),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
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
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
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
