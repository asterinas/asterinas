// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, fmt::Debug, string::ToString, sync::Arc, vec::Vec};
use core::hint::spin_loop;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use aster_frame::{
    io_mem::IoMem,
    sync::{RwLock, SpinLock},
    trap::TrapFrame,
    vm::{Daddr, DmaDirection, HasDaddr, VmReader},
};
use aster_network::{DmaPool, DmaSegment};
use aster_util::safe_ptr::SafePtr;
use log::debug;
use spin::Once;

use super::{config::VirtioConsoleConfig, DEVICE_NAME};
use crate::{
    device::{console::config::ConsoleFeatures, VirtioDeviceError},
    dma_buf::DmaBuf,
    queue::VirtQueue,
    transport::VirtioTransport,
};

static SEND_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();
static RECV_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();

pub struct ConsoleDevice {
    config: SafePtr<VirtioConsoleConfig, IoMem>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    receive_queue: SpinLock<VirtQueue>,
    transmit_queue: SpinLock<VirtQueue>,
    receive_buffers: Vec<DmaSegment>,
    transmit_buffers: SpinLock<Vec<Option<DmaSegment>>>,
    callbacks: RwLock<Vec<&'static ConsoleCallback>>,
}

impl AnyConsoleDevice for ConsoleDevice {
    fn send(&self, value: &[u8]) {
        let mut reader = VmReader::from(value);

        while reader.has_remain() {
            if self.try_send(&mut reader).is_err() {
                // TODO: do we want to use wait queue to wait until send queue is available?
                // This helps avoid using `spin_loop` but will cause `println` sleepable.
                spin_loop();
                continue;
            }
        }
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        self.callbacks.write_irq_disabled().push(callback);
    }
}

struct DmaSegmentSlice<'a> {
    segment: &'a DmaSegment,
    offset: usize,
    len: usize,
}

impl<'a> DmaSegmentSlice<'a> {
    fn new(segment: &'a DmaSegment, offset: usize, len: usize) -> Self {
        assert!(offset + len <= segment.len());
        Self {
            segment,
            offset,
            len,
        }
    }
}

impl<'a> HasDaddr for DmaSegmentSlice<'a> {
    fn daddr(&self) -> Daddr {
        self.segment.daddr() + self.offset
    }
}

impl<'a> DmaBuf for DmaSegmentSlice<'a> {
    fn len(&self) -> usize {
        self.len
    }
}

impl Debug for ConsoleDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConsoleDevice")
            .field("config", &self.config)
            .field("transport", &self.transport)
            .field("receive_queue", &self.receive_queue)
            .field("transmit_queue", &self.transmit_queue)
            .finish()
    }
}

impl ConsoleDevice {
    pub fn negotiate_features(features: u64) -> u64 {
        let mut features = ConsoleFeatures::from_bits_truncate(features);
        // A virtio console device may have multiple ports, but we only use one port to communicate now.
        features.remove(ConsoleFeatures::VIRTIO_CONSOLE_F_MULTIPORT);
        features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config = VirtioConsoleConfig::new(transport.as_ref());
        const RECV0_QUEUE_INDEX: u16 = 0;
        const TRANSMIT0_QUEUE_INDEX: u16 = 1;

        // The virtio queues for virtio-console have max size of 2
        const RECV_QUEUE_SIZE: u16 = 2;
        const TRANSMIT_QUEUE_SIZE: u16 = 2;
        const SEND_BUF_SIZE: u16 = 256;
        const RECV_BUF_SIZE: u16 = 256;

        let receive_queue = SpinLock::new(
            VirtQueue::new(RECV0_QUEUE_INDEX, RECV_QUEUE_SIZE, transport.as_mut()).unwrap(),
        );
        let transmit_queue = SpinLock::new(
            VirtQueue::new(
                TRANSMIT0_QUEUE_INDEX,
                TRANSMIT_QUEUE_SIZE,
                transport.as_mut(),
            )
            .unwrap(),
        );

        SEND_BUFFER_POOL.call_once(|| {
            DmaPool::new(SEND_BUF_SIZE as usize, 1, 16, DmaDirection::ToDevice, false)
        });
        let transmit_buffers = (0..TRANSMIT_QUEUE_SIZE)
            .map(|_| None::<DmaSegment>)
            .collect::<Vec<_>>();

        RECV_BUFFER_POOL.call_once(|| {
            DmaPool::new(
                RECV_BUF_SIZE as usize,
                1,
                1,
                DmaDirection::FromDevice,
                false,
            )
        });
        let receive_buffers = {
            let mut receive_queue = receive_queue.lock_irq_disabled();
            (0..RECV_QUEUE_SIZE)
                .map(|i| {
                    let buffer = {
                        let pool = RECV_BUFFER_POOL
                            .get()
                            .expect("RECV_BUFFER_POOL is not initialized");
                        pool.alloc_segment()
                            .expect("Fails to allocate receive buffer")
                    };
                    let token = receive_queue
                        .add_dma_buf(&[], &[&buffer])
                        .expect("Fails to add receive buffer");
                    assert_eq!(token, i);

                    if receive_queue.should_notify() {
                        receive_queue.notify();
                    }

                    buffer
                })
                .collect()
        };

        let device = Arc::new(Self {
            config,
            transport: SpinLock::new(transport),
            receive_queue,
            transmit_queue,
            receive_buffers,
            transmit_buffers: SpinLock::new(transmit_buffers),
            callbacks: RwLock::new(Vec::new()),
        });

        // Register irq callbacks
        let mut transport = device.transport.lock_irq_disabled();
        let handle_console_input = {
            let device = device.clone();
            move |_: &TrapFrame| device.handle_recv_irq()
        };
        transport
            .register_queue_callback(RECV0_QUEUE_INDEX, Box::new(handle_console_input), false)
            .unwrap();
        transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        transport.finish_init();
        drop(transport);

        aster_console::register_device(DEVICE_NAME.to_string(), device);

        Ok(())
    }

    fn try_send(&self, reader: &mut VmReader) -> Result<(), aster_frame::Error> {
        // Lock order: transmit_buffers -> transmit_queue
        let mut transmit_buffers = self.transmit_buffers.lock_irq_disabled();
        let mut transmit_queue = self.transmit_queue.lock_irq_disabled();

        while reader.has_remain() {
            if !free_processed_transmit_buffer(&mut transmit_buffers, &mut transmit_queue) {
                // TODO: Use wait instead of spin loop in the future
                return Err(aster_frame::Error::NotEnoughResources);
            }

            let segment = {
                let pool = SEND_BUFFER_POOL
                    .get()
                    .expect("SEND_BUFFER_POOL is not initialized");
                pool.alloc_segment().expect("Fails to allocate block")
            };

            let segment_slice = {
                let mut writer = segment.writer().unwrap();
                let len = writer.write(reader);
                segment.sync(0..len).unwrap();

                DmaSegmentSlice::new(&segment, 0, len)
            };

            let token = transmit_queue
                .add_dma_buf(&[&segment_slice], &[])
                .expect("Fails to add dma buffer");
            if transmit_queue.should_notify() {
                transmit_queue.notify();
            }

            let buffer = transmit_buffers
                .get_mut(token as usize)
                .expect("Invalid token");
            debug_assert!(buffer.is_none());
            *buffer = Some(segment);
        }

        Ok(())
    }

    fn handle_recv_irq(&self) {
        let mut receive_queue = self.receive_queue.lock_irq_disabled();
        let callbacks = self.callbacks.read_irq_disabled();
        while receive_queue.can_pop() {
            let (token, len) = receive_queue.pop_used().unwrap();

            let block = self.receive_buffers.get(token as usize).unwrap();
            block.sync(0..len as usize).unwrap();

            for callback in callbacks.iter() {
                let reader = block.reader().unwrap().limit(len as usize);
                callback(reader);
            }

            let new_token = receive_queue.add_dma_buf(&[], &[block]).unwrap();
            assert_eq!(new_token, token);

            if receive_queue.should_notify() {
                receive_queue.notify();
            }
        }
    }
}

fn config_space_change(_: &TrapFrame) {
    debug!("Virtio-Console device configuration space change");
}

fn free_processed_transmit_buffer(
    transmit_buffers: &mut [Option<DmaSegment>],
    transmit_queue: &mut VirtQueue,
) -> bool {
    while transmit_queue.can_pop() {
        let (token, _) = transmit_queue.pop_used().expect("Pop used buffer");
        let buffer = transmit_buffers
            .get_mut(token as usize)
            .expect("Fails to get transmit buffer");
        debug_assert!(buffer.is_some());
        *buffer = None;
    }

    transmit_queue.available_desc() > 0
}
