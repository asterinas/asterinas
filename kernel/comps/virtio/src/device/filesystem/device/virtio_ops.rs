// SPDX-License-Identifier: MPL-2.0

//! Virtio-fs device initialization and virtqueue interrupt handling.
//!
//! This module connects [`FileSystemDevice`] to the generic virtio transport:
//! it negotiates feature bits, creates high-priority and request queues, and
//! dispatches virtqueue completions to the request queue layer.

use alloc::{boxed::Box, string::ToString, sync::Arc, vec::Vec};
use core::cmp;

use ostd::{arch::trap::TrapFrame, debug, info};

use super::{
    DEFAULT_QUEUE_SIZE, FileSystemDevice, HIPRIO_QUEUE_INDEX,
    queue::{FsRequestQueue, MAX_DMA_BUFS_PER_REQUEST},
    register_device,
};
use crate::{
    device::{
        VirtioDeviceError,
        filesystem::{
            DEVICE_NAME,
            config::{FileSystemFeatures, VirtioFsConfig},
        },
    },
    queue::VirtQueue,
    transport::VirtioTransport,
};

impl FileSystemDevice {
    /// Negotiates the feature bits supported by the virtio-fs driver.
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let device_features = FileSystemFeatures::from_bits_truncate(features);
        let supported_features = FileSystemFeatures::supported_features();
        let fs_features = device_features & supported_features;
        debug!("features negotiated: {:?}", fs_features);
        fs_features.bits()
    }

    /// Initializes one virtio-fs device from its virtio transport.
    pub(crate) fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioFsConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();

        let negotiated_features = FileSystemFeatures::from_bits_truncate(Self::negotiate_features(
            transport.read_device_features(),
        ));
        let notify_supported = negotiated_features.contains(FileSystemFeatures::NOTIFICATION);
        // Queue layout:
        // - Queue 0: high-priority queue (always present);
        // - Queue 1: notification queue (present only if notifications were negotiated);
        // - Remaining queues: request queues.
        let request_queue_start_idx = if notify_supported { 2 } else { 1 };

        let total_queues = transport.num_queues();
        if total_queues <= request_queue_start_idx {
            return Err(VirtioDeviceError::UnsupportedConfig);
        }

        let max_request_queue_count = (total_queues - request_queue_start_idx) as usize;
        let request_queue_count = cmp::min(
            config.num_request_queues() as usize,
            max_request_queue_count,
        );

        if request_queue_count == 0 {
            return Err(VirtioDeviceError::UnsupportedConfig);
        }

        let device = {
            let hiprio_queue =
                FsRequestQueue::new(Self::new_queue(HIPRIO_QUEUE_INDEX, transport.as_mut())?);

            let mut request_queues = Vec::with_capacity(request_queue_count);
            for idx in 0..request_queue_count {
                let queue_index = request_queue_start_idx + idx as u16;
                request_queues.push(FsRequestQueue::new(Self::new_queue(
                    queue_index,
                    transport.as_mut(),
                )?));
            }

            Arc::new(Self::new(
                transport,
                hiprio_queue,
                request_queues,
                config.parse_tag().to_string(),
                notify_supported,
            ))
        };

        // Register completion taskless.
        device.init_completion_taskless();

        // Register configuration callback.
        let mut transport = device.transport.lock();
        transport.register_cfg_callback(Box::new(|_: &TrapFrame| {
            debug!("Virtio-FS device configuration space changed");
        }))?;

        // Register the callback for the high-priority queue.
        let hiprio_queue_callback = {
            let device = device.clone();
            move |_: &TrapFrame| {
                device.handle_queue_irq(device.hiprio_queue.as_ref());
            }
        };
        transport.register_queue_callback(
            HIPRIO_QUEUE_INDEX,
            Box::new(hiprio_queue_callback),
            false,
        )?;

        // TODO: Register the callback for the notification queue, if present.

        // Register the callback for each request queue.
        for idx in 0..request_queue_count {
            let queue_index = request_queue_start_idx + idx as u16;

            let request_queue_callback = {
                let device = device.clone();
                move |_: &TrapFrame| {
                    device.handle_queue_irq(device.request_queues[idx].as_ref());
                }
            };
            transport.register_queue_callback(
                queue_index,
                Box::new(request_queue_callback),
                false,
            )?;
        }

        transport.finish_init();
        drop(transport);

        register_device(device.clone());

        info!(
            "{} initialized, tag = {}, request_queues = {}, notify = {}",
            DEVICE_NAME,
            device.tag.as_str(),
            device.request_queues.len(),
            device.notify_supported
        );
        Ok(())
    }

    fn handle_queue_irq(&self, request_queue: &FsRequestQueue) {
        request_queue.drain_completed_requests();
        request_queue.schedule_completion_taskless();
    }

    fn new_queue(
        index: u16,
        transport: &mut dyn VirtioTransport,
    ) -> Result<VirtQueue, VirtioDeviceError> {
        let max_queue_size = transport
            .max_queue_size(index)
            .map_err(VirtioDeviceError::from)?;
        let queue_size = DEFAULT_QUEUE_SIZE.min(max_queue_size);

        if queue_size < MAX_DMA_BUFS_PER_REQUEST as u16 {
            return Err(VirtioDeviceError::UnsupportedConfig);
        }

        VirtQueue::new(index, queue_size, transport).map_err(Into::into)
    }
}
