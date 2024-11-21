// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, fmt::Debug, string::ToString, sync::Arc, vec::Vec};
use core::hint::spin_loop;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use log::debug;
use ostd::{
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmReader},
    sync::{LocalIrqDisabled, RwLock, SpinLock},
    trap::TrapFrame,
};

use super::{config::VirtioConsoleConfig, DEVICE_NAME};
use crate::{
    device::{console::config::ConsoleFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};

pub struct ConsoleDevice {
    config_manager: ConfigManager<VirtioConsoleConfig>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    receive_queue: SpinLock<VirtQueue>,
    transmit_queue: SpinLock<VirtQueue>,
    send_buffer: DmaStream,
    receive_buffer: DmaStream,
    callbacks: RwLock<Vec<&'static ConsoleCallback>, LocalIrqDisabled>,
}

impl AnyConsoleDevice for ConsoleDevice {
    fn send(&self, value: &[u8]) {
        let mut transmit_queue = self.transmit_queue.disable_irq().lock();
        let mut reader = VmReader::from(value);

        while reader.remain() > 0 {
            let mut writer = self.send_buffer.writer().unwrap();
            let len = writer.write(&mut reader);
            self.send_buffer.sync(0..len).unwrap();

            let slice = DmaStreamSlice::new(&self.send_buffer, 0, len);
            transmit_queue.add_dma_buf(&[&slice], &[]).unwrap();

            if transmit_queue.should_notify() {
                transmit_queue.notify();
            }
            while !transmit_queue.can_pop() {
                spin_loop();
            }
            transmit_queue.pop_used().unwrap();
        }
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        self.callbacks.write().push(callback);
    }
}

impl Debug for ConsoleDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConsoleDevice")
            .field("config", &self.config_manager.read_config())
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
        let config_manager = VirtioConsoleConfig::new_manager(transport.as_ref());
        debug!("virtio_console_config = {:?}", config_manager.read_config());

        const RECV0_QUEUE_INDEX: u16 = 0;
        const TRANSMIT0_QUEUE_INDEX: u16 = 1;
        let receive_queue =
            SpinLock::new(VirtQueue::new(RECV0_QUEUE_INDEX, 2, transport.as_mut()).unwrap());
        let transmit_queue =
            SpinLock::new(VirtQueue::new(TRANSMIT0_QUEUE_INDEX, 2, transport.as_mut()).unwrap());

        let send_buffer = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::ToDevice, false).unwrap()
        };

        let receive_buffer = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::FromDevice, false).unwrap()
        };

        let device = Arc::new(Self {
            config_manager,
            transport: SpinLock::new(transport),
            receive_queue,
            transmit_queue,
            send_buffer,
            receive_buffer,
            callbacks: RwLock::new(Vec::new()),
        });

        device.activate_receive_buffer(&mut device.receive_queue.disable_irq().lock());

        // Register irq callbacks
        let mut transport = device.transport.disable_irq().lock();
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

    fn handle_recv_irq(&self) {
        let mut receive_queue = self.receive_queue.disable_irq().lock();

        let Ok((_, len)) = receive_queue.pop_used() else {
            return;
        };
        self.receive_buffer.sync(0..len as usize).unwrap();

        let callbacks = self.callbacks.read();
        for callback in callbacks.iter() {
            let reader = self.receive_buffer.reader().unwrap().limit(len as usize);
            callback(reader);
        }
        drop(callbacks);

        self.activate_receive_buffer(&mut receive_queue);
    }

    fn activate_receive_buffer(&self, receive_queue: &mut VirtQueue) {
        receive_queue
            // We limit the buffer length to one to work around a QEMU bug that causes incorrect
            // results when pasting more than 32 bytes into the virtio console. This has no
            // performance penalty, since QEMU always gets one byte at a time, regardless of
            // whether we have this limit or not.
            //
            // For the QEMU bug, see details at
            // <https://lore.kernel.org/qemu-devel/20240707111940.232549-3-lrh2000@pku.edu.cn/T/#u>.
            .add_dma_buf(&[], &[&DmaStreamSlice::new(&self.receive_buffer, 0, 1)])
            .unwrap();

        if receive_queue.should_notify() {
            receive_queue.notify();
        }
    }
}

fn config_space_change(_: &TrapFrame) {
    debug!("Virtio-Console device configuration space change");
}
