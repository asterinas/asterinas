// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use aster_block::{
    BlockDeviceMeta,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio, bio_segment_pool_init},
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use aster_util::safe_ptr::SafePtr;
use log::info;
use ostd::{
    mm::{DmaCoherent, FrameAllocOptions, HasDaddr},
    sync::SpinLock,
};

use crate::{
    NVMePciTransport, NVMeRegs32, NVMeRegs64,
    device::{MAX_NS_NUM, NVMeDeviceError, NVMeNamespace, NVMeStats},
    nvme_cmd,
    nvme_queue::{NVMeCompletionQueue, NVMeSubmissionQueue, QUEUE_NUM},
    nvme_regs::NVMeDoorBellRegs,
};

pub const BLOCK_SIZE: usize = ostd::mm::PAGE_SIZE;

#[derive(Debug)]
pub struct NVMeBlockDevice {
    device: Arc<NVMeDeviceInner>,
    queue: BioRequestSingleQueue,
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
}

impl NVMeBlockDevice {
    pub(crate) fn init(transport: NVMePciTransport) -> Result<(), NVMeDeviceError> {
        info!("[NVMe]: Block device starts to initialize!");
        let device = NVMeDeviceInner::init(transport)?;

        let device_id = "nvme0".to_string();

        let block_device = Arc::new(Self {
            device: device.clone(),
            queue: BioRequestSingleQueue::with_max_nr_segments_per_bio(
                NVMeDeviceInner::QUEUE_SIZE as usize,
            ),
        });

        device.reset_controller();

        device.configure_admin_queue();

        device.set_entry_size();

        device.enable_controller();

        device.identify_controller();

        device.identify_ns_list();

        device.identify_ns(1);

        device.create_io_queues();

        aster_block::register_device(device_id, block_device);

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

#[derive(Debug)]
pub struct NVMeDeviceInner {
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

    pub fn init(transport: NVMePciTransport) -> Result<Arc<Self>, NVMeDeviceError> {
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

    pub fn reset_controller(&self) {
        let transport = self.transport.lock();
        transport.reset_controller();
    }

    pub fn configure_admin_queue(&self) {
        let transport = self.transport.lock();
        let acq = &self.completion_queues[0].disable_irq().lock();
        let asq = &self.submission_queues[0].disable_irq().lock();

        let _ = transport.write_reg32(
            NVMeRegs32::Aqa,
            ((acq.length() - 1) << 16) | (asq.length() - 1),
        );
        let _ = transport.write_reg64(NVMeRegs64::Asq, asq.sq_paddr() as u64);
        let _ = transport.write_reg64(NVMeRegs64::Acq, acq.cq_paddr() as u64);
    }

    pub fn set_entry_size(&self) {
        let transport = self.transport.lock();
        transport.set_entry_size();
    }

    pub fn enable_controller(&self) {
        let transport = self.transport.lock();
        transport.enable_controller();
    }

    pub fn identify_controller(&self) {
        let data: SafePtr<u8, DmaCoherent> = SafePtr::new(
            DmaCoherent::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                true,
            )
            .unwrap(),
            0,
        );

        {
            let qid = 0;
            let mut queue = self.submission_queues[qid].disable_irq().lock();
            let cid = queue.tail();
            let entry = nvme_cmd::identify_controller(cid, data.paddr());
            let tail = queue.submit(entry);
            self.submission_queue_tail_update(qid as u16, tail as u32);
        }

        {
            let qid = 0;
            let mut queue = self.completion_queues[qid].disable_irq().lock();
            let (head, _entry, _) = queue.complete_spin();
            self.completion_queue_head_update(qid as u16, head as u32);
        }

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

        info!(
            "[NVMe]: Model: {}; Serial: {}; Firmware: {}",
            model.trim(),
            serial.trim(),
            firmware.trim()
        );
    }

    pub fn identify_ns_list(&self) {
        let data: SafePtr<u32, DmaCoherent> = SafePtr::new(
            DmaCoherent::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                true,
            )
            .unwrap(),
            0,
        );

        {
            let qid = 0;
            let mut queue = self.submission_queues[qid].disable_irq().lock();
            let cid = queue.tail();
            let entry = nvme_cmd::identify_namespace_list(cid, data.paddr(), 1);
            let tail = queue.submit(entry);
            self.submission_queue_tail_update(qid as u16, tail as u32);
        }

        {
            let qid = 0;
            let mut queue = self.completion_queues[qid].disable_irq().lock();
            let (head, _entry, _) = queue.complete_spin();
            self.completion_queue_head_update(qid as u16, head as u32);
        }

        let mut result = [0u32; MAX_NS_NUM];
        data.read_slice(&mut result).unwrap();

        let mut nsids = Vec::new();
        for &nsid in result.iter() {
            if nsid != 0 {
                nsids.push(nsid);
            }
        }
        info!("[NVMe]: Device has {} namespaces", nsids.len());
    }

    pub fn identify_ns(&self, nsid: u32) {
        let data: SafePtr<u64, DmaCoherent> = SafePtr::new(
            DmaCoherent::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                true,
            )
            .unwrap(),
            0,
        );

        {
            let qid = 0;
            let mut queue = self.submission_queues[qid].disable_irq().lock();
            let cid = queue.tail();
            let entry = nvme_cmd::identify_namespace(cid, data.paddr(), nsid);
            let tail = queue.submit(entry);
            self.submission_queue_tail_update(qid as u16, tail as u32);
        }

        {
            let qid = 0;
            let mut queue = self.completion_queues[qid].disable_irq().lock();
            let (head, _entry, _) = queue.complete_spin();
            self.completion_queue_head_update(qid as u16, head as u32);
        }

        let mut result = [0u64; 3];
        data.read_slice(&mut result).unwrap();

        let size = result[0];
        let capacity = result[1];
        let used = result[2];
        let block_size = 512;
        info!(
            "[NVMe]: ID: {}; Size: {}; Capacity: {}; Used: {}",
            nsid,
            size * block_size,
            capacity * block_size,
            used * block_size,
        );

        self.namespaces.disable_irq().lock().push(NVMeNamespace {
            id: nsid,
            free_blocks: size,
            used_blocks: used,
            block_size,
        });
    }

    pub fn create_io_queues(&self) {
        for io_qid in 1..self.queue_num {
            let (cptr, clength) = {
                let cqueue = &self.completion_queues[io_qid].disable_irq().lock();
                (cqueue.cq_paddr(), cqueue.length())
            };

            {
                let qid = 0;
                let mut queue = self.submission_queues[qid].disable_irq().lock();
                let cid = queue.tail();
                let entry = nvme_cmd::create_io_completion_queue(
                    cid,
                    io_qid as u16,
                    cptr,
                    (clength - 1) as u16,
                );
                let tail = queue.submit(entry);
                self.submission_queue_tail_update(qid as u16, tail as u32);
            }

            {
                let qid = 0;
                let mut queue = self.completion_queues[qid].disable_irq().lock();
                let (head, _entry, _) = queue.complete_spin();
                self.completion_queue_head_update(qid as u16, head as u32);
            }

            let (sptr, slen) = {
                let squeue = &self.submission_queues[io_qid].disable_irq().lock();
                (squeue.sq_paddr(), squeue.length())
            };

            {
                let qid = 0;
                let mut queue = self.submission_queues[qid].disable_irq().lock();
                let cid = queue.tail();
                let entry = nvme_cmd::create_io_submission_queue(
                    cid,
                    io_qid as u16,
                    sptr,
                    (slen - 1) as u16,
                    io_qid as u16,
                );
                let tail = queue.submit(entry);
                self.submission_queue_tail_update(qid as u16, tail as u32);
            }

            {
                let qid = 0;
                let mut queue = self.completion_queues[qid].disable_irq().lock();
                let (head, _entry, _) = queue.complete_spin();
                self.completion_queue_head_update(qid as u16, head as u32);
            }
            info!(
                "[NVMe]: Finish creating submission queue {io_qid} and completion queue {io_qid}"
            );
        }
    }

    pub fn read_dbreg(&self, reg: NVMeDoorBellRegs, qid: u16) -> u32 {
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

    pub fn write_dbreg(&self, reg: NVMeDoorBellRegs, qid: u16, val: u32) {
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

    pub fn read(&self, request: BioRequest) {
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
            .stream()
            .daddr()
            .try_into()
            .unwrap();

        while blocks_num > 0 {
            let once_blocks_num = if blocks_num < Self::PRP1_BLOCK_NUM {
                blocks_num
            } else {
                Self::PRP1_BLOCK_NUM
            };
            let ptr1 = 0;

            info!(
                "[NVMe]: Handling read command, with lba: {lba}, blocks_num: {blocks_num}, ptr0: {ptr0}"
            );

            {
                let qid = 1;
                let mut queue = self.submission_queues[qid].disable_irq().lock();
                let cid = queue.tail();
                let entry = nvme_cmd::io_read(cid, nsid, lba, once_blocks_num - 1, ptr0, ptr1);
                let tail = queue.submit(entry);
                self.submission_queue_tail_update(qid as u16, tail as u32);
                self.stats.disable_irq().lock().submitted += 1;
            }

            {
                let qid = 1;
                let mut queue = self.completion_queues[qid].disable_irq().lock();
                let (head, _entry, _) = queue.complete_spin();
                self.completion_queue_head_update(qid as u16, head as u32);
                self.stats.disable_irq().lock().completed += 1;
            }

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

    pub fn write(&self, request: BioRequest) {
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
            .stream()
            .daddr()
            .try_into()
            .unwrap();

        while blocks_num > 0 {
            let once_blocks_num = if blocks_num < Self::PRP1_BLOCK_NUM {
                blocks_num
            } else {
                Self::PRP1_BLOCK_NUM
            };

            let ptr1 = 0;

            info!(
                "[NVMe]: Handling write command, with lba: {lba}, blocks_num: {blocks_num}, ptr0: {ptr0}"
            );

            {
                let qid = 1;
                let mut queue = self.submission_queues[qid].disable_irq().lock();
                let cid = queue.tail();
                let entry = nvme_cmd::io_write(cid, nsid, lba, once_blocks_num - 1, ptr0, ptr1);
                let tail = queue.submit(entry);
                self.submission_queue_tail_update(qid as u16, tail as u32);
                self.stats.disable_irq().lock().submitted += 1;
            }

            {
                let qid = 1;
                let mut queue = self.completion_queues[qid].disable_irq().lock();
                let (head, _entry, _) = queue.complete_spin();
                self.completion_queue_head_update(qid as u16, head as u32);
                self.stats.disable_irq().lock().completed += 1;
            }

            lba += once_blocks_num as u64;
            blocks_num -= once_blocks_num;
            ptr0 += 512 * once_blocks_num as u64;
        }

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }

    pub fn flush(&self, request: BioRequest) {
        let nsid = 1;

        info!("[NVMe]: Handling flush command");
        {
            let qid = 1;
            let mut queue = self.submission_queues[qid].disable_irq().lock();
            let cid = queue.tail();
            let entry = nvme_cmd::io_flush(cid, nsid);
            let tail = queue.submit(entry);
            self.submission_queue_tail_update(qid as u16, tail as u32);
            self.stats.disable_irq().lock().submitted += 1;
        }

        {
            let qid = 1;
            let mut queue = self.completion_queues[qid].disable_irq().lock();
            let (head, _entry, _) = queue.complete_spin();
            self.completion_queue_head_update(qid as u16, head as u32);
            self.stats.disable_irq().lock().completed += 1;
        }

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
        mm::{UntypedMem, VmIo},
        prelude::ktest,
    };

    use super::*;
    use crate::nvme_init;

    const TEST_CHAR: u8 = b'B';
    const TEST_BUF_LENGTH: usize = 8192;

    #[ktest]
    fn initialize() {
        component::init_all(component::parse_metadata!()).unwrap();
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
        let bio_segment;

        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(buf_nblocks)
            .unwrap();
        if bio_type == BioType::Write {
            segment.writer().fill(val);
            bio_segment = BioSegment::new_from_segment(segment.into(), BioDirection::ToDevice);
        } else if bio_type == BioType::Read {
            bio_segment = BioSegment::new_from_segment(segment.into(), BioDirection::FromDevice);
        } else {
            panic!("Now only Read and Write requests could be created.");
        }

        let bio = Bio::new(
            bio_type,
            Sid::from(Bid::from_offset(0 as _)),
            vec![bio_segment.clone()],
            None,
        );
        let _ = bio.submit(device);
        bio_segment
    }

    #[ktest]
    fn write_then_read() {
        let device_name = "nvme0";
        let device = aster_block::get_device(device_name).expect("NVMe device not found");
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
