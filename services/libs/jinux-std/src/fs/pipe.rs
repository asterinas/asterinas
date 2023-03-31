use crate::prelude::*;

use super::file_handle::File;
use super::utils::{Consumer, IoEvents, Poller, Producer};

pub struct PipeReader {
    consumer: Consumer<u8>,
}

impl PipeReader {
    pub fn new(consumer: Consumer<u8>) -> Self {
        Self { consumer }
    }
}

impl File for PipeReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let is_nonblocking = self.consumer.is_nonblocking();

        // Fast path
        let res = self.consumer.read(buf);
        if should_io_return(&res, is_nonblocking) {
            return res;
        }

        // Slow path
        let mask = IoEvents::IN;
        let poller = Poller::new();
        loop {
            let res = self.consumer.read(buf);
            if should_io_return(&res, is_nonblocking) {
                return res;
            }
            let events = self.poll(mask, Some(&poller));
            if events.is_empty() {
                poller.wait();
            }
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.consumer.poll(mask, poller)
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

impl File for PipeWriter {
    fn write(&self, buf: &[u8]) -> Result<usize> {
        let is_nonblocking = self.producer.is_nonblocking();

        // Fast path
        let res = self.producer.write(buf);
        if should_io_return(&res, is_nonblocking) {
            return res;
        }

        // Slow path
        let mask = IoEvents::OUT;
        let poller = Poller::new();
        loop {
            let res = self.producer.write(buf);
            if should_io_return(&res, is_nonblocking) {
                return res;
            }
            let events = self.poll(mask, Some(&poller));
            if events.is_empty() {
                poller.wait();
            }
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.producer.poll(mask, poller)
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
