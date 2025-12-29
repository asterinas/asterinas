// SPDX-License-Identifier: MPL-2.0

//! NVMe Block Device implementation.
//!
//! This module implements the block device interface for NVMe storage devices,
//! following the NVM Express Base Specification Revision 2.0.

use alloc::{format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use aster_block::{
    BlockDeviceMeta,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio, bio_segment_pool_init},
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use aster_util::safe_ptr::SafePtr;
use device_id::DeviceId;
use log::info;
use ostd::{
    mm::{DmaCoherent, FrameAllocOptions, HasDaddr},
    sync::SpinLock,
};

use crate::{
    NVME_BLOCK_MAJOR_ID, NVMePciTransport, NVMeRegs32, NVMeRegs64,
    device::{MAX_NS_NUM, NVMeDeviceError, NVMeNamespace, NVMeStats},
    nvme_cmd::{self, NVMeCommand},
    nvme_queue::{NVMeCompletionQueue, NVMeSubmissionQueue, QUEUE_NUM},
    nvme_regs::NVMeDoorBellRegs,
};

pub(crate) const BLOCK_SIZE: usize = ostd::mm::PAGE_SIZE;

#[derive(Debug)]
pub struct NVMeBlockDevice {
    device: Arc<NVMeDeviceInner>,
    queue: BioRequestSingleQueue,
    name: String,
    id: DeviceId,
}

impl aster_block::BlockDevice for NVMeBlockDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        self.queue.enqueue(bio)
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: self.queue.max_nr_segments_per_bio(),
            nr_sectors: self.device.namespaces.disable_irq().lock()[0].block_size as usize,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn id(&self) -> DeviceId {
        self.id
    }
}

static NR_NVME_DEVICE: AtomicU32 = AtomicU32::new(0);

impl NVMeBlockDevice {
    pub(crate) fn init(transport: NVMePciTransport) -> Result<(), NVMeDeviceError> {
        let device = NVMeDeviceInner::init(transport)?;

        let index = NR_NVME_DEVICE.fetch_add(1, Ordering::Relaxed);
        let name = formatted_device_name(index);
        // Use the allocated major ID for the NVMe device
        let major_id = NVME_BLOCK_MAJOR_ID.get().unwrap().get();
        let id = DeviceId::new(major_id, device_id::MinorId::new(index));

        let block_device = Arc::new(Self {
            device: device.clone(),
            queue: BioRequestSingleQueue::with_max_nr_segments_per_bio(
                NVMeDeviceInner::QUEUE_SIZE as usize,
            ),
            name,
            id,
        });

        device.reset_controller();

        device.configure_admin_queue();

        device.set_entry_size();

        device.enable_controller();

        device.identify_controller();

        let nsids = device.identify_ns_list();

        for nsid in nsids {
            device.identify_ns(nsid);
        }

        device.create_io_queues();

        aster_block::register(block_device).unwrap();

        bio_segment_pool_init();
        Ok(())
    }

    /// Dequeues a `BioRequest` from the software staging queue and
    /// processes the request.
    pub fn handle_requests(&self) {
        let request = self.queue.dequeue();
        info!("[NVMe]: Handle Request: {:?}", request);
        match request.type_() {
            BioType::Read => self.device.read(request),
            BioType::Write => self.device.write(request),
            BioType::Flush => self.device.flush(request),
            BioType::Discard => todo!(),
        }
    }
}

fn formatted_device_name(index: u32) -> String {
    format!("nvme{}", index)
}

#[derive(Debug)]
pub(crate) struct NVMeDeviceInner {
    submission_queues: [SpinLock<NVMeSubmissionQueue>; QUEUE_NUM],
    completion_queues: [SpinLock<NVMeCompletionQueue>; QUEUE_NUM],
    queue_num: usize,
    dstrd: u16,
    namespaces: SpinLock<Vec<NVMeNamespace>>,
    transport: SpinLock<NVMePciTransport>,
    stats: SpinLock<NVMeStats>,
}

impl NVMeDeviceInner {
    /// PRP1 points to the first physical page, which contains at most 8 blocks.
    const PRP1_BLOCK_NUM: u16 = 8;
    const QUEUE_SIZE: u16 = 64;

    pub(crate) fn init(transport: NVMePciTransport) -> Result<Arc<Self>, NVMeDeviceError> {
        let device = Arc::new(NVMeDeviceInner {
            submission_queues: [
                SpinLock::new(NVMeSubmissionQueue::new().unwrap()),
                SpinLock::new(NVMeSubmissionQueue::new().unwrap()),
            ],
            completion_queues: [
                SpinLock::new(NVMeCompletionQueue::new().unwrap()),
                SpinLock::new(NVMeCompletionQueue::new().unwrap()),
            ],
            queue_num: QUEUE_NUM,
            dstrd: ((transport.read_reg64(NVMeRegs64::Cap) >> 32) & 0b1111) as u16,
            namespaces: SpinLock::new(Vec::new()),
            transport: SpinLock::new(transport),
            stats: SpinLock::new(NVMeStats {
                submitted: 0,
                completed: 0,
            }),
        });

        Ok(device)
    }

    pub(crate) fn reset_controller(&self) {
        let transport = self.transport.lock();
        transport.reset_controller();
    }

    pub(crate) fn configure_admin_queue(&self) {
        let transport = self.transport.lock();
        let acq = &self.completion_queues[0].disable_irq().lock();
        let asq = &self.submission_queues[0].disable_irq().lock();

        let _ = transport.write_reg32(
            NVMeRegs32::Aqa,
            ((acq.length() - 1) << 16) | (asq.length() - 1),
        );
        let _ = transport.write_reg64(NVMeRegs64::Asq, asq.sq_daddr() as u64);
        let _ = transport.write_reg64(NVMeRegs64::Acq, acq.cq_daddr() as u64);
    }

    pub(crate) fn set_entry_size(&self) {
        let transport = self.transport.lock();
        transport.set_entry_size();
    }

    pub(crate) fn enable_controller(&self) {
        let transport = self.transport.lock();
        transport.enable_controller();
    }

    pub(crate) fn identify_controller(&self) {
        let data: SafePtr<u8, DmaCoherent> = SafePtr::new(
            DmaCoherent::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                true,
            )
            .unwrap(),
            0,
        );

        let qid = 0;
        let cid = {
            let queue = self.submission_queues[qid].disable_irq().lock();
            queue.tail()
        };
        let entry = nvme_cmd::identify_controller(cid, data.daddr());
        self.submit_and_wait(qid, entry);

        let mut result = [0u8; 128];
        data.read_slice(&mut result).unwrap();

        let mut serial = String::new();
        for &b in &result[4..24] {
            if b == 0 {
                break;
            }
            serial.push(b as char);
        }

        let mut model = String::new();
        for &b in &result[24..64] {
            if b == 0 {
                break;
            }
            model.push(b as char);
        }

        let mut firmware = String::new();
        for &b in &result[64..72] {
            if b == 0 {
                break;
            }
            firmware.push(b as char);
        }
    }

    pub(crate) fn identify_ns_list(&self) -> Vec<u32> {
        let data: SafePtr<u32, DmaCoherent> = SafePtr::new(
            DmaCoherent::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                true,
            )
            .unwrap(),
            0,
        );

        let qid = 0;
        let cid = {
            let queue = self.submission_queues[qid].disable_irq().lock();
            queue.tail()
        };
        let entry = nvme_cmd::identify_namespace_list(cid, data.daddr(), 1);
        self.submit_and_wait(qid, entry);

        let mut result = [0u32; MAX_NS_NUM];
        data.read_slice(&mut result).unwrap();

        let mut nsids = Vec::new();
        for &nsid in result.iter() {
            if nsid != 0 {
                nsids.push(nsid);
            }
        }
        nsids
    }

    pub(crate) fn identify_ns(&self, nsid: u32) {
        let data: SafePtr<u64, DmaCoherent> = SafePtr::new(
            DmaCoherent::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                true,
            )
            .unwrap(),
            0,
        );

        let qid = 0;
        let cid = {
            let queue = self.submission_queues[qid].disable_irq().lock();
            queue.tail()
        };
        let entry = nvme_cmd::identify_namespace(cid, data.daddr(), nsid);
        self.submit_and_wait(qid, entry);

        let mut result = [0u64; 3];
        data.read_slice(&mut result).unwrap();

        let size = result[0];
        let _capacity = result[1];
        let used = result[2];
        let block_size = 512;

        self.namespaces.disable_irq().lock().push(NVMeNamespace {
            id: nsid,
            free_blocks: size,
            used_blocks: used,
            block_size,
        });
    }

    pub(crate) fn create_io_queues(&self) {
        let qid = 0;
        for io_qid in 1..self.queue_num {
            let (cptr, clength) = {
                let cqueue = &self.completion_queues[io_qid].disable_irq().lock();
                (cqueue.cq_daddr(), cqueue.length())
            };

            let cid = {
                let queue = self.submission_queues[qid].disable_irq().lock();
                queue.tail()
            };
            let entry = nvme_cmd::create_io_completion_queue(
                cid,
                io_qid as u16,
                cptr,
                (clength - 1) as u16,
            );
            self.submit_and_wait(qid, entry);

            let (sptr, slen) = {
                let squeue = &self.submission_queues[io_qid].disable_irq().lock();
                (squeue.sq_daddr(), squeue.length())
            };

            let cid = {
                let queue = self.submission_queues[qid].disable_irq().lock();
                queue.tail()
            };
            let entry = nvme_cmd::create_io_submission_queue(
                cid,
                io_qid as u16,
                sptr,
                (slen - 1) as u16,
                io_qid as u16,
            );
            self.submit_and_wait(qid, entry);
        }
    }

    pub(crate) fn read_dbreg(&self, reg: NVMeDoorBellRegs, qid: u16) -> u32 {
        let transport = self.transport.lock();
        match reg {
            NVMeDoorBellRegs::Sqtdb => transport
                .config_bar
                .read_once(
                    (0x1000 + ((4 << self.dstrd) * (2 * qid)) as u32)
                        .try_into()
                        .unwrap(),
                )
                .unwrap(),
            NVMeDoorBellRegs::Cqhdb => transport
                .config_bar
                .read_once(
                    (0x1000 + ((4 << self.dstrd) * (2 * qid + 1)) as u32)
                        .try_into()
                        .unwrap(),
                )
                .unwrap(),
        }
    }

    pub(crate) fn write_dbreg(&self, reg: NVMeDoorBellRegs, qid: u16, val: u32) {
        let transport = self.transport.lock();
        match reg {
            NVMeDoorBellRegs::Sqtdb => {
                let _ = transport.config_bar.write_once(
                    (0x1000 + ((4 << self.dstrd) * (2 * qid)) as u32)
                        .try_into()
                        .unwrap(),
                    val,
                );
            }
            NVMeDoorBellRegs::Cqhdb => {
                let _ = transport.config_bar.write_once(
                    (0x1000 + ((4 << self.dstrd) * (2 * qid + 1)) as u32)
                        .try_into()
                        .unwrap(),
                    val,
                );
            }
        }
    }

    fn submission_queue_tail_update(&self, qid: u16, tail: u32) {
        self.write_dbreg(NVMeDoorBellRegs::Sqtdb, qid, tail);
    }

    fn completion_queue_head_update(&self, qid: u16, head: u32) {
        self.write_dbreg(NVMeDoorBellRegs::Cqhdb, qid, head);
    }

    /// Submits a command to the submission queue and waits for its completion.
    ///
    /// This is a common pattern used by all NVMe commands: submit the command,
    /// update the doorbell, wait for completion, and update the completion queue head.
    fn submit_and_wait(&self, qid: usize, entry: NVMeCommand) {
        {
            let mut queue = self.submission_queues[qid].disable_irq().lock();
            let tail = queue.submit(entry);
            self.submission_queue_tail_update(qid as u16, tail as u32);
        }

        {
            let mut queue = self.completion_queues[qid].disable_irq().lock();
            let (head, _entry, _) = queue.complete_spin();
            self.completion_queue_head_update(qid as u16, head as u32);
        }
    }

    pub(crate) fn read(&self, request: BioRequest) {
        let nsid = 1;
        let mut lba = request.sid_range().start.to_raw();
        let mut blocks_num = request.num_sectors() as u16;
        let mut ptr0 = request
            .bios()
            .next()
            .unwrap()
            .segments()
            .first()
            .unwrap()
            .inner_dma_slice()
            .daddr()
            .try_into()
            .unwrap();

        let qid = 1;
        while blocks_num > 0 {
            let once_blocks_num = if blocks_num < Self::PRP1_BLOCK_NUM {
                blocks_num
            } else {
                Self::PRP1_BLOCK_NUM
            };
            let ptr1 = 0;

            let cid = {
                let queue = self.submission_queues[qid].disable_irq().lock();
                queue.tail()
            };
            let entry = nvme_cmd::io_read(cid, nsid, lba, once_blocks_num - 1, ptr0, ptr1);
            self.submit_and_wait(qid, entry);
            self.stats.disable_irq().lock().submitted += 1;
            self.stats.disable_irq().lock().completed += 1;

            lba += once_blocks_num as u64;
            blocks_num -= once_blocks_num;
            ptr0 += 512 * once_blocks_num as u64;
        }

        request
            .bios()
            .flat_map(|bio| {
                bio.segments()
                    .iter()
                    .map(|segment| segment.inner_dma_slice())
            })
            .for_each(|dma_slice| dma_slice.sync().unwrap());

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }

    pub(crate) fn write(&self, request: BioRequest) {
        let nsid = 1;
        let mut lba = request.sid_range().start.to_raw();
        let mut blocks_num = request.num_sectors() as u16;
        let mut ptr0 = request
            .bios()
            .next()
            .unwrap()
            .segments()
            .first()
            .unwrap()
            .inner_dma_slice()
            .daddr()
            .try_into()
            .unwrap();

        let qid = 1;
        while blocks_num > 0 {
            let once_blocks_num = if blocks_num < Self::PRP1_BLOCK_NUM {
                blocks_num
            } else {
                Self::PRP1_BLOCK_NUM
            };

            let ptr1 = 0;

            let cid = {
                let queue = self.submission_queues[qid].disable_irq().lock();
                queue.tail()
            };
            let entry = nvme_cmd::io_write(cid, nsid, lba, once_blocks_num - 1, ptr0, ptr1);
            self.submit_and_wait(qid, entry);
            self.stats.disable_irq().lock().submitted += 1;
            self.stats.disable_irq().lock().completed += 1;

            lba += once_blocks_num as u64;
            blocks_num -= once_blocks_num;
            ptr0 += 512 * once_blocks_num as u64;
        }

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }

    pub(crate) fn flush(&self, request: BioRequest) {
        let nsid = 1;
        let qid = 1;

        let cid = {
            let queue = self.submission_queues[qid].disable_irq().lock();
            queue.tail()
        };
        let entry = nvme_cmd::io_flush(cid, nsid);
        self.submit_and_wait(qid, entry);
        self.stats.disable_irq().lock().submitted += 1;
        self.stats.disable_irq().lock().completed += 1;

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use aster_block::{
        bio::{Bio, BioDirection, BioSegment},
        id::{Bid, Sid},
    };
    use ostd::{
        mm::{VmIo, VmReader, io_util::HasVmReaderWriter},
        prelude::ktest,
    };

    use super::*;
    use crate::nvme_init;

    const TEST_CHAR: u8 = b'B';
    const TEST_BUF_LENGTH: usize = 8192;

    #[ktest]
    fn initialize() {
        component::init_all(
            component::InitStage::Bootstrap,
            component::parse_metadata!(),
        )
        .unwrap();
        let result = nvme_init();
        assert!(result.is_ok(), "NVMe_init() returned an error");
    }

    fn create_and_submit_bio_request(
        device: &NVMeBlockDevice,
        bio_type: BioType,
        buf_len: usize,
        val: u8,
    ) -> BioSegment {
        let buf_nblocks = buf_len / BLOCK_SIZE;
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(buf_nblocks)
            .unwrap();

        if bio_type == BioType::Write {
            let mut writer = segment.writer();
            let fill_buf = [val; BLOCK_SIZE];
            for _ in 0..buf_nblocks {
                let mut reader = VmReader::from(fill_buf.as_slice());
                writer.write(&mut reader);
            }
        }

        let direction = match bio_type {
            BioType::Write => BioDirection::ToDevice,
            BioType::Read => BioDirection::FromDevice,
            _ => panic!("Now only Read and Write requests could be created."),
        };
        let bio_segment = BioSegment::new_from_segment(segment.into(), direction);

        let bio = Bio::new(
            bio_type,
            Sid::from(Bid::from_offset(0)),
            vec![bio_segment.clone()],
            None,
        );
        let _ = bio.submit(device);
        bio_segment
    }

    #[ktest]
    fn write_then_read() {
        let device = aster_block::collect_all()
            .into_iter()
            .find(|d| d.name() == "nvme0")
            .expect("NVMe device not found");
        let device_arc = Arc::clone(&device);

        let nvme_block_device = device_arc
            .downcast_ref::<NVMeBlockDevice>()
            .expect("Failed to downcast device");

        let mut read_buf = [0u8; TEST_BUF_LENGTH];
        let val = TEST_CHAR;
        let _ =
            create_and_submit_bio_request(nvme_block_device, BioType::Write, TEST_BUF_LENGTH, val);
        nvme_block_device.handle_requests();
        let read_bio_segment =
            create_and_submit_bio_request(nvme_block_device, BioType::Read, TEST_BUF_LENGTH, val);
        nvme_block_device.handle_requests();

        read_bio_segment
            .inner_dma_slice()
            .read_bytes(0, &mut read_buf)
            .unwrap();

        assert!(read_buf.iter().all(|&x| x == TEST_CHAR));
    }
}
