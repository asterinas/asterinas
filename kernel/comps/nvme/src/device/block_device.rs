// SPDX-License-Identifier: MPL-2.0

//! NVMe Block Device implementation.
//!
//! This module implements the block device interface for NVMe storage devices,
//! following the NVM Express Base Specification Revision 2.0.

use alloc::{format, string::String, sync::Arc, vec::Vec};
use core::{
    hint::spin_loop,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_block::{
    BlockDeviceMeta, SECTOR_SIZE,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio, bio_segment_pool_init},
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use aster_util::safe_ptr::SafePtr;
use device_id::DeviceId;
use log::{debug, info};
use ostd::{
    Pod,
    mm::{FrameAllocOptions, HasDaddr, dma::DmaStream},
    sync::{LocalIrqDisabled, SpinLock, WaitQueue, Waiter},
    task::Task,
};

use crate::{
    NVME_BLOCK_MAJOR_ID, NVMePciTransport, NvmeRegs32, NvmeRegs64,
    device::{MAX_NS_NUM, NVMeDeviceError, NVMeNamespace, NVMeStats},
    nvme_cmd::{self, NVMeCommand, NVMeCompletion},
    nvme_queue::{NVMeCompletionQueue, NVMeSubmissionQueue, QUEUE_NUM},
    nvme_regs::NvmeDoorBellRegs,
};

#[derive(Debug)]
pub struct NVMeBlockDevice {
    device: NVMeDeviceInner,
    queue: BioRequestSingleQueue,
    name: String,
    id: DeviceId,
}

impl aster_block::BlockDevice for NVMeBlockDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        self.queue.enqueue(bio)
    }

    fn metadata(&self) -> BlockDeviceMeta {
        let state = self.device.state.lock();
        let ns = state
            .namespace
            .as_ref()
            .expect("NVMe namespace should be initialized during device init");

        BlockDeviceMeta {
            max_nr_segments_per_bio: self.queue.max_nr_segments_per_bio(),
            nr_sectors: ns.block_size as usize,
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
            device,
            queue: BioRequestSingleQueue::with_max_nr_segments_per_bio(
                NVMeDeviceInner::QUEUE_SIZE as usize,
            ),
            name,
            id,
        });

        // NVMe controller initialization sequence.
        //
        // See NVMe Spec 2.0, Section 3.5 (Controller Initialization).
        //   1. Wait for CSTS.RDY to become '0' (controller ready to be reset)
        //   2. Configure Admin Queue by setting AQA, ASQ, and ACQ
        //   3. Set I/O queue entry sizes (CC.IOSQES and CC.IOCQES)
        //   4. Enable the controller by setting CC.EN to '1'
        //   5. Wait for CSTS.RDY to become '1' (controller ready to process commands)
        //   6. Optionally configure MSI-X interrupts for the admin queue
        {
            let mut state = block_device.device.state.lock();
            let mut ctx = InitContext::new(&mut state, &block_device.device);

            ctx.reset_controller();
            ctx.configure_admin_queue();
            ctx.set_entry_size();
            ctx.enable_controller();

            if block_device.device.has_msix && Task::current().is_some() {
                ctx.setup_admin_msix_handler(&block_device);
            }
        }

        block_device.device.identify_controller();

        let nsids = block_device.device.identify_ns_list();
        if nsids.is_empty() {
            log::error!("[NVMe]: No namespaces found on this device");
            return Err(NVMeDeviceError::NoNamespace);
        }

        block_device.device.identify_ns(nsids[0]);

        block_device.device.create_io_queues(&block_device);

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

#[derive(Copy, Clone, Pod)]
#[repr(C)]
struct IdentifyControllerData {
    _reserved: [u8; 4],
    serial: [u8; 20],
    model: [u8; 40],
    firmware: [u8; 8],
    _rest: [u8; 56],
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
struct IdentifyNamespaceListData {
    nsids: [u32; MAX_NS_NUM],
}

#[derive(Copy, Clone, Pod)]
#[repr(C)]
struct IdentifyNamespaceData {
    size: u64,
    capacity: u64,
    used: u64,
}

struct DeviceState {
    transport: NVMePciTransport,
    namespace: Option<NVMeNamespace>,
}

struct InitContext<'a> {
    state: &'a mut DeviceState,
    device: &'a NVMeDeviceInner,
}

impl<'a> InitContext<'a> {
    fn new(state: &'a mut DeviceState, device: &'a NVMeDeviceInner) -> Self {
        Self { state, device }
    }
}

pub(crate) struct NVMeDeviceInner {
    submission_queues: [SpinLock<NVMeSubmissionQueue, LocalIrqDisabled>; QUEUE_NUM],
    completion_queues: [SpinLock<NVMeCompletionQueue, LocalIrqDisabled>; QUEUE_NUM],
    completion_wait_queues: [WaitQueue; QUEUE_NUM],
    queue_num: usize,
    dstrd: u16,
    state: SpinLock<DeviceState, LocalIrqDisabled>,
    stats: NVMeStats,
    has_msix: bool,
}

impl core::fmt::Debug for NVMeDeviceInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NVMeDeviceInner")
            .field("queue_num", &self.queue_num)
            .field("dstrd", &self.dstrd)
            .field("has_msix", &self.has_msix)
            .finish()
    }
}

impl NVMeDeviceInner {
    /// PRP1 points to the first physical page, which contains at most 8 blocks.
    const PRP1_BLOCK_NUM: u16 = 8;
    const QUEUE_SIZE: u16 = 64;

    const CAP_REG_HIGH_DWORD_SHIFT: u32 = 32;
    const DSTRD_MASK: u64 = 0b1111;

    pub(crate) fn init(transport: NVMePciTransport) -> Result<Self, NVMeDeviceError> {
        let has_msix = transport.has_msix();
        let dstrd = ((transport.read_reg64(NvmeRegs64::Cap) >> Self::CAP_REG_HIGH_DWORD_SHIFT)
            & Self::DSTRD_MASK) as u16;

        let device = NVMeDeviceInner {
            submission_queues: [
                SpinLock::new(NVMeSubmissionQueue::new().unwrap()),
                SpinLock::new(NVMeSubmissionQueue::new().unwrap()),
            ],
            completion_queues: [
                SpinLock::new(NVMeCompletionQueue::new().unwrap()),
                SpinLock::new(NVMeCompletionQueue::new().unwrap()),
            ],
            completion_wait_queues: [WaitQueue::new(), WaitQueue::new()],
            queue_num: QUEUE_NUM,
            dstrd,
            state: SpinLock::new(DeviceState {
                transport,
                namespace: None,
            }),
            stats: NVMeStats::new(),
            has_msix,
        };

        Ok(device)
    }
}

impl<'a> InitContext<'a> {
    /// Controller Configuration Enable bit.
    const NVME_CC_ENABLE: u32 = 0x1;
    /// Controller Status Ready bit.
    const NVME_CSTS_RDY: u32 = 0x1;
    /// I/O Submission Queue Entry Size bits.
    const IOSQES_BITS: u32 = 20;
    /// I/O Submission Queue Entry Size value.
    const IOSQES_VALUE: u32 = 4;
    /// I/O Completion Queue Entry Size bits.
    const IOCQES_BITS: u32 = 16;
    /// I/O Completion Queue Entry Size value.
    const IOCQES_VALUE: u32 = 6;

    fn reset_controller(&mut self) {
        let mut cc = self.state.transport.read_reg32(NvmeRegs32::Cc);
        cc &= !Self::NVME_CC_ENABLE;
        self.state.transport.write_reg32(NvmeRegs32::Cc, cc);

        loop {
            let csts = self.state.transport.read_reg32(NvmeRegs32::Csts);
            if (csts & Self::NVME_CSTS_RDY) == 0 {
                break;
            }
            spin_loop();
        }
    }

    fn configure_admin_queue(&mut self) {
        let acq = &self.device.completion_queues[0].lock();
        let asq = &self.device.submission_queues[0].lock();

        self.state.transport.write_reg32(
            NvmeRegs32::Aqa,
            ((acq.length() - 1) << 16) | (asq.length() - 1),
        );
        self.state
            .transport
            .write_reg64(NvmeRegs64::Asq, asq.sq_daddr() as u64);
        self.state
            .transport
            .write_reg64(NvmeRegs64::Acq, acq.cq_daddr() as u64);
    }

    fn set_entry_size(&mut self) {
        let mut cc = self.state.transport.read_reg32(NvmeRegs32::Cc);
        cc = cc
            | (Self::IOSQES_VALUE << Self::IOSQES_BITS)
            | (Self::IOCQES_VALUE << Self::IOCQES_BITS);
        self.state.transport.write_reg32(NvmeRegs32::Cc, cc);
    }

    fn enable_controller(&mut self) {
        let mut cc = self.state.transport.read_reg32(NvmeRegs32::Cc);
        cc |= Self::NVME_CC_ENABLE;
        self.state.transport.write_reg32(NvmeRegs32::Cc, cc);

        loop {
            let csts = self.state.transport.read_reg32(NvmeRegs32::Csts);
            if (csts & Self::NVME_CSTS_RDY) == 1 {
                break;
            }
            spin_loop();
        }
    }

    fn setup_admin_msix_handler(&mut self, block_device: &Arc<NVMeBlockDevice>) {
        if self.device.has_msix
            && Task::current().is_some()
            && let Some(msix_manager) = self.state.transport.msix_manager_mut()
        {
            // Setup admin queue interrupt (vector 0)
            let (_admin_vec, admin_irq) = msix_manager.admin_irq();
            let device_weak = Arc::downgrade(block_device);
            admin_irq.on_active(move |_| {
                if let Some(block_device) = device_weak.upgrade() {
                    block_device.device.process_admin_completions();
                }
            });
        }
    }
}

impl NVMeDeviceInner {
    pub(crate) fn identify_controller(&self) {
        let data: SafePtr<IdentifyControllerData, DmaStream> = SafePtr::new(
            DmaStream::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                false,
            )
            .unwrap(),
            0,
        );

        let qid = 0;
        let cid = {
            let queue = self.submission_queues[qid].lock();
            queue.tail()
        };
        let entry = nvme_cmd::identify_controller(cid, data.daddr());
        self.submit_and_wait(qid, entry);

        let result = data.read().unwrap();

        let mut serial = String::new();
        for &b in &result.serial {
            if b == 0 {
                break;
            }
            serial.push(b as char);
        }

        let mut model = String::new();
        for &b in &result.model {
            if b == 0 {
                break;
            }
            model.push(b as char);
        }

        let mut firmware = String::new();
        for &b in &result.firmware {
            if b == 0 {
                break;
            }
            firmware.push(b as char);
        }

        debug!(
            "[NVMe]: Controller identified - Serial: {}, Model: {}, Firmware: {}",
            serial, model, firmware
        );
    }

    pub(crate) fn identify_ns_list(&self) -> Vec<u32> {
        let data: SafePtr<IdentifyNamespaceListData, DmaStream> = SafePtr::new(
            DmaStream::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                false,
            )
            .unwrap(),
            0,
        );

        let qid = 0;
        let cid = {
            let queue = self.submission_queues[qid].lock();
            queue.tail()
        };
        let entry = nvme_cmd::identify_namespace_list(cid, data.daddr(), 0);
        self.submit_and_wait(qid, entry);

        let result = data.read().unwrap();

        let mut nsids = Vec::new();
        for &nsid in result.nsids.iter() {
            if nsid != 0 {
                nsids.push(nsid);
            }
        }
        nsids
    }

    pub(crate) fn identify_ns(&self, nsid: u32) {
        let data: SafePtr<IdentifyNamespaceData, DmaStream> = SafePtr::new(
            DmaStream::map(
                FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                false,
            )
            .unwrap(),
            0,
        );

        let qid = 0;
        let cid = {
            let queue = self.submission_queues[qid].lock();
            queue.tail()
        };
        let entry = nvme_cmd::identify_namespace(cid, data.daddr(), nsid);
        self.submit_and_wait(qid, entry);

        let result = data.read().unwrap();

        self.state.lock().namespace = Some(NVMeNamespace {
            id: nsid,
            free_blocks: result.size,
            used_blocks: result.used,
            block_size: result.capacity,
        });
    }

    pub(crate) fn create_io_queues(&self, block_device: &Arc<NVMeBlockDevice>) {
        let qid = 0;

        let mut msix_vectors: Vec<Option<u16>> = Vec::new();
        if self.has_msix && Task::current().is_some() {
            let mut state = self.state.lock();
            if let Some(msix_manager) = state.transport.msix_manager_mut() {
                for io_qid in 1..self.queue_num {
                    if let Some((vector, io_irq)) = msix_manager.alloc_io_queue_irq() {
                        // Setup interrupt handler immediately after allocation
                        let device_weak = Arc::downgrade(block_device);
                        let handler_qid = io_qid;
                        io_irq.on_active(move |_| {
                            if let Some(block_device) = device_weak.upgrade() {
                                block_device.device.process_io_completions(handler_qid);
                            }
                        });
                        msix_vectors.push(Some(vector));
                    } else {
                        log::warn!(
                            "[NVMe]: Failed to allocate MSI-X vector for I/O queue {}",
                            io_qid
                        );
                        msix_vectors.push(None);
                        break;
                    }
                }
            }
        }

        for io_qid in 1..self.queue_num {
            let (cptr, clength) = {
                let cqueue = &self.completion_queues[io_qid].lock();
                (cqueue.cq_daddr(), cqueue.length())
            };

            let cid = {
                let queue = self.submission_queues[qid].lock();
                queue.tail()
            };

            // Get MSI-X vector for this queue (if available)
            let msix_vector = if self.has_msix && Task::current().is_some() {
                let idx = io_qid - 1;
                if idx < msix_vectors.len() {
                    msix_vectors[idx]
                } else {
                    None
                }
            } else {
                None
            };

            let entry = nvme_cmd::create_io_completion_queue(
                cid,
                io_qid as u16,
                cptr,
                (clength - 1) as u16,
                msix_vector,
            );
            self.submit_and_wait_polling(qid, entry);

            let (sptr, slen) = {
                let squeue = &self.submission_queues[io_qid].lock();
                (squeue.sq_daddr(), squeue.length())
            };

            let cid = {
                let queue = self.submission_queues[qid].lock();
                queue.tail()
            };
            let entry = nvme_cmd::create_io_submission_queue(
                cid,
                io_qid as u16,
                sptr,
                (slen - 1) as u16,
                io_qid as u16,
            );
            self.submit_and_wait_polling(qid, entry);
        }
    }

    fn process_admin_completions(&self) {
        let qid = 0;
        self.completion_wait_queues[qid].wake_all();
    }

    fn process_io_completions(&self, qid: usize) {
        if qid == 0 || qid >= self.queue_num {
            log::error!(
                "[NVMe]: Invalid queue ID {} for I/O completion processing",
                qid
            );
            return;
        }
        self.completion_wait_queues[qid].wake_all();
    }

    #[expect(dead_code)]
    pub(crate) fn read_dbreg(&self, reg: NvmeDoorBellRegs, qid: u16) -> u32 {
        let state = self.state.lock();
        let offset = reg.offset(qid, self.dstrd);
        state
            .transport
            .config_bar
            .read_once(offset.try_into().unwrap())
            .unwrap()
    }

    pub(crate) fn write_dbreg(&self, reg: NvmeDoorBellRegs, qid: u16, val: u32) {
        let state = self.state.lock();
        let offset = reg.offset(qid, self.dstrd);
        state
            .transport
            .config_bar
            .write_once(offset.try_into().unwrap(), val)
            .unwrap();
    }

    fn submission_queue_tail_update(&self, qid: u16, tail: u32) {
        self.write_dbreg(NvmeDoorBellRegs::Sqtdb, qid, tail);
    }

    fn completion_queue_head_update(&self, qid: u16, head: u32) {
        self.write_dbreg(NvmeDoorBellRegs::Cqhdb, qid, head);
    }

    /// Submits a command to the submission queue and waits for its completion.
    fn submit_and_wait(&self, qid: usize, entry: NVMeCommand) {
        if self.has_msix && Task::current().is_some() {
            let (waiter, _) = Waiter::new_pair();
            let wait_queue = &self.completion_wait_queues[qid];
            wait_queue.enqueue(waiter.waker());

            // Get the actual cid while holding the lock to avoid race conditions
            let cid = {
                let mut sq = self.submission_queues[qid].lock();
                let cid = sq.tail(); // The cid is the current tail before submit
                let tail = sq.submit(entry);
                self.submission_queue_tail_update(qid as u16, tail as u32);
                cid
            };

            // Hybrid approach: brief polling first, then interrupt-based wait
            // Most NVMe commands complete very quickly, so polling is often sufficient
            // If not completed after polling, fall back to interrupt-based waiting
            const POLL_SPINS: u32 = 1000;
            for _ in 0..POLL_SPINS {
                let mut cq = self.completion_queues[qid].lock();
                if let Some((new_head, completion, _old_head)) = cq.complete()
                    && self.process_completion(qid, new_head, completion, cid)
                {
                    return;
                }
                core::hint::spin_loop();
            }

            // If not completed after polling, wait for interrupt
            loop {
                {
                    let mut cq = self.completion_queues[qid].lock();
                    if let Some((new_head, completion, _old_head)) = cq.complete()
                        && self.process_completion(qid, new_head, completion, cid)
                    {
                        return;
                    }
                }

                wait_queue.enqueue(waiter.waker());
                waiter.wait();
            }
        } else {
            self.submit_and_wait_polling(qid, entry);
        }
    }

    fn submit_and_wait_polling(&self, qid: usize, entry: NVMeCommand) {
        {
            let mut queue = self.submission_queues[qid].lock();
            let tail = queue.submit(entry);
            self.submission_queue_tail_update(qid as u16, tail as u32);
        }

        let mut queue = self.completion_queues[qid].lock();
        let (head, _entry, _) = queue.complete_spin();
        self.completion_queue_head_update(qid as u16, head as u32);
    }

    /// Processes a single completion entry and returns whether it matches the expected CID.
    /// Returns `true` if the completion matches `expected_cid`.
    fn process_completion(
        &self,
        qid: usize,
        new_head: u16,
        completion: NVMeCompletion,
        expected_cid: u16,
    ) -> bool {
        let is_target = completion.cid == expected_cid;

        self.completion_queue_head_update(qid as u16, new_head as u32);

        if qid > 0 {
            self.stats.increment_completed();
        }

        if completion.has_error() {
            log::error!(
                "[NVMe]: Command failed: CID={}, Status={:04X} (SC={}), SQID={}, QID={}",
                completion.cid,
                completion.status,
                completion.status_code(),
                completion.sq_id,
                qid
            );
        }

        is_target
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

            // cid will be set by submit_and_wait, passing 0 as placeholder
            let entry = nvme_cmd::io_read(0, nsid, lba, once_blocks_num - 1, ptr0, ptr1);
            self.submit_and_wait(qid, entry);
            self.stats.increment_submitted();
            self.stats.increment_completed();

            lba += once_blocks_num as u64;
            blocks_num -= once_blocks_num;
            ptr0 += (SECTOR_SIZE as u64) * once_blocks_num as u64;
        }

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

            // cid will be set by submit_and_wait, passing 0 as placeholder
            let entry = nvme_cmd::io_write(0, nsid, lba, once_blocks_num - 1, ptr0, ptr1);
            self.submit_and_wait(qid, entry);
            self.stats.increment_submitted();
            self.stats.increment_completed();

            lba += once_blocks_num as u64;
            blocks_num -= once_blocks_num;
            ptr0 += (SECTOR_SIZE as u64) * once_blocks_num as u64;
        }

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }

    pub(crate) fn flush(&self, request: BioRequest) {
        let nsid = 1;
        let qid = 1;

        let cid = {
            let queue = self.submission_queues[qid].lock();
            queue.tail()
        };
        let entry = nvme_cmd::io_flush(cid, nsid);
        self.submit_and_wait(qid, entry);
        self.stats.increment_submitted();
        self.stats.increment_completed();

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }
}

#[cfg(ktest)]
mod test {
    use alloc::{sync::Arc, vec};

    use aster_block::{
        BLOCK_SIZE,
        bio::{Bio, BioDirection, BioSegment},
        id::{Bid, Sid},
    };
    use ostd::{
        mm::{FrameAllocOptions, VmIo, VmReader, io_util::HasVmReaderWriter},
        prelude::ktest,
    };

    use super::{BioType, NVMeBlockDevice};
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
        assert!(result.is_ok(), "NVMe_init returned an error");
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
        let _ = bio.submit(device).unwrap();
        bio_segment
    }

    #[ktest]
    fn write_then_read() {
        if aster_block::collect_all().is_empty() {
            component::init_all(
                component::InitStage::Bootstrap,
                component::parse_metadata!(),
            )
            .unwrap();
        }

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
