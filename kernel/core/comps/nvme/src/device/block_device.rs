// SPDX-License-Identifier: MPL-2.0

//! NVMe Block Device implementation.
//!
//! Implements the [`aster_block::BlockDevice`] trait on top of the NVMe transport.
//! BIOs are staged in [`BioRequestSingleQueue`] and drained by repeated calls to
//! [`NvmeBlockDevice::handle_requests`] from the kernel registry's per-device kthread.
//!
//! Each in-flight hardware command is stored in the I/O submission queue's per-slot
//! context under its CID. One [`BioRequest`] may require several NVMe commands (MDTS /
//! NLB limits), so those CIDs share one [`InFlightRequest`] that completes the whole
//! `BioRequest` when the last command finishes.

use alloc::{borrow::ToOwned, format, string::String, sync::Arc, vec, vec::Vec};
use core::{
    ffi::CStr,
    hint::spin_loop,
    num::NonZeroUsize,
    sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};

use aster_block::{
    BlockDeviceMeta, SECTOR_SIZE,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio, bio_segment_pool_init},
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use aster_util::safe_ptr::SafePtr;
use device_id::DeviceId;
use ostd::{
    debug, error, info,
    mm::{HasDaddr, HasSize, PAGE_SIZE, dma::DmaStream},
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard, WaitQueue},
    timer::Jiffies,
    warn,
};

use super::{
    namespace::{LBA_SIZE, NvmeNamespace},
    stat::NvmeStats,
};
use crate::{
    NVME_BLOCK_MAJOR_ID,
    device::{MAX_NS_NUM, NvmeDeviceError},
    nvme_cmd,
    nvme_queue::{
        NvmeCompletionQueue, NvmeCompletionQueueAccess, NvmeSubmissionQueue,
        NvmeSubmissionQueueAccess, QUEUE_DEPTH, QUEUE_NUM,
    },
    nvme_regs::{NvmeRegs32, NvmeRegs64},
    nvme_spec::NvmeCommand,
    prp::PrpPointers,
    transport::pci::transport::{NvmePciTransport, NvmePciTransportLock},
};

/// Admin submission and completion queue pair (NVMe queue ID 0).
const ADMIN_QID: usize = 0;
/// I/O submission and completion queue pair (NVMe queue ID 1).
const IO_QID: usize = 1;
// TODO: Support multiple I/O submission and completion queue pairs.
ostd::const_assert!(IO_QID + 1 == QUEUE_NUM);

#[derive(Debug)]
pub struct NvmeBlockDevice {
    device: NvmeDeviceInner,
    queue: BioRequestSingleQueue,
    name: String,
    id: DeviceId,
}

static NR_NVME_DEVICE: AtomicU32 = AtomicU32::new(0);

impl NvmeBlockDevice {
    pub(crate) fn init(transport: NvmePciTransport) -> Result<(), NvmeDeviceError> {
        let (device, io_msix_vectors) = NvmeDeviceInner::init(transport)?;

        let index = NR_NVME_DEVICE.fetch_add(1, Ordering::Relaxed);
        let name = formatted_device_name(index, device.namespace.id);
        let id = {
            // Use the allocated major ID for the NVMe device
            let major_id = NVME_BLOCK_MAJOR_ID.get().unwrap().get();
            DeviceId::new(major_id, device_id::MinorId::new(index))
        };

        let block_device = Arc::new(Self {
            device,
            queue: BioRequestSingleQueue::new(),
            name,
            id,
        });

        block_device
            .device
            .setup_msix_handlers(&block_device, io_msix_vectors);

        aster_block::register(block_device)
            .map_err(|_| NvmeDeviceError::BlockDeviceRegisterFailed)?;

        bio_segment_pool_init();
        Ok(())
    }

    /// Dequeues a `BioRequest` from the software staging queue and
    /// processes the request.
    pub fn handle_requests(&self) {
        let request = self.queue.dequeue();
        debug!("Handle Request: {:?}", request);
        match request.type_() {
            BioType::Read => self.device.read(request),
            BioType::Write => self.device.write(request),
            BioType::Flush => self.device.flush(request),
        }
    }

    /// Returns the highest number of concurrently in-flight I/O commands observed so far.
    #[cfg(ktest)]
    fn max_in_flight_commands(&self) -> u64 {
        self.device.stats.max_in_flight()
    }

    /// Resets I/O command counters used to measure queue depth.
    #[cfg(ktest)]
    fn reset_io_stats(&self) {
        self.device.stats.reset_stats();
    }
}

impl aster_block::BlockDevice for NvmeBlockDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        self.queue.enqueue(bio)
    }

    fn metadata(&self) -> BlockDeviceMeta {
        const { assert!(LBA_SIZE == SECTOR_SIZE) };
        let sectors = self.device.namespace.nsze;

        BlockDeviceMeta {
            max_nr_segments_per_bio: self.queue.max_nr_segments_per_bio(),
            nr_sectors: sectors as usize,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn id(&self) -> DeviceId {
        self.id
    }
}

fn formatted_device_name(index: u32, nsid: u32) -> String {
    format!("nvme{}n{}", index, nsid)
}

struct NvmeDeviceInner {
    submission_queues:
        [SpinLock<NvmeSubmissionQueue<SubmittedRequest>, LocalIrqDisabled>; QUEUE_NUM],
    completion_queues: [SpinLock<NvmeCompletionQueue, LocalIrqDisabled>; QUEUE_NUM],
    completion_wait_queues: [WaitQueue; QUEUE_NUM],
    transport: NvmePciTransportLock,
    namespace: NvmeNamespace,
    dstrd: u16,
    max_io_bytes: NonZeroUsize,
    stats: NvmeStats,
}

#[derive(Clone, Copy)]
enum IoOp {
    Read,
    Write,
}

/// Shared state for one outstanding [`BioRequest`] that may span multiple NVMe commands.
///
/// Large requests are split across several NVMe commands, so multiple
/// [`SubmittedRequest`] entries share this tracker until `remaining` reaches zero.
struct InFlightRequest {
    bio_request: SpinLock<Option<BioRequest>, LocalIrqDisabled>,
    remaining: AtomicUsize,
    failed: AtomicBool,
}

impl InFlightRequest {
    fn finish(self: Arc<Self>) {
        let status = if self.failed.load(Ordering::Relaxed) {
            BioStatus::IoError
        } else {
            BioStatus::Complete
        };
        if let Some(bio_request) = self.bio_request.lock().take() {
            bio_request.into_bios().for_each(|bio| {
                bio.complete(status);
            });
        }
    }
}

/// Per-command completion context stored until the matching CQE arrives.
///
/// The table key is the command's CID, and `_prp` keeps PRP list pages alive for
/// the command lifetime.
struct SubmittedRequest {
    request: Arc<InFlightRequest>,
    _prp: Option<PrpPointers>,
}

struct BuiltIoCommand {
    entry: NvmeCommand,
    prp: PrpPointers,
}

impl core::fmt::Debug for NvmeDeviceInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmeDeviceInner")
            .field("namespace", &self.namespace)
            .field("dstrd", &self.dstrd)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl NvmeDeviceInner {
    const CAP_TO_UNIT_MILLIS: u64 = 500;
    const MPS_BASE_PAGE_SHIFT: u32 = 12;

    fn init(mut transport: NvmePciTransport) -> Result<(Self, IoMsixVectors), NvmeDeviceError> {
        let cap = transport.regs().read64(NvmeRegs64::Cap);

        let dstrd = ((cap >> NvmeRegs64::CAP_DSTRD_SHIFT) & NvmeRegs64::CAP_DSTRD_MASK) as u16;

        let cap_mpsmin =
            ((cap >> NvmeRegs64::CAP_MPSMIN_SHIFT) & NvmeRegs64::CAP_MPSMIN_MASK) as u32;
        let cap_mpsmax =
            ((cap >> NvmeRegs64::CAP_MPSMAX_SHIFT) & NvmeRegs64::CAP_MPSMAX_MASK) as u32;
        let cc_mps_value =
            const { PAGE_SIZE.trailing_zeros() - NvmeDeviceInner::MPS_BASE_PAGE_SHIFT };
        if cc_mps_value < cap_mpsmin || cc_mps_value > cap_mpsmax {
            error!(
                "Host page size not supported by controller: host MPS={}, CAP.MPSMIN={}, CAP.MPSMAX={}",
                cc_mps_value, cap_mpsmin, cap_mpsmax
            );
            return Err(NvmeDeviceError::InvalidControllerConfig);
        }

        let timeout_units = (cap >> NvmeRegs64::CAP_TO_SHIFT) & NvmeRegs64::CAP_TO_MASK;
        // CAP.TO is defined in 500 ms units; treat zero as one unit so we still
        // have a bounded wait on devices that report 0.
        let timeout_millis = timeout_units.max(1) * Self::CAP_TO_UNIT_MILLIS;
        let controller_ready_timeout = Duration::from_millis(timeout_millis);

        let mut init_ctx =
            InitContext::new(transport, dstrd, cc_mps_value, controller_ready_timeout)?;

        // NVMe controller initialization sequence.
        //
        // See NVMe Spec 2.0, Section 3.5 (Controller Initialization).
        //   1. Wait for CSTS.RDY to become '0' (controller ready to be reset)
        //   2. Configure Admin Queue by setting AQA, ASQ, and ACQ
        //   3. Set I/O queue entry sizes (CC.IOSQES and CC.IOCQES)
        //   4. Enable the controller by setting CC.EN to '1'
        //   5. Wait for CSTS.RDY to become '1' (controller ready to process commands)
        //   6. Configure MSI-X interrupts for the Admin Queue
        init_ctx.reset_controller()?;
        init_ctx.configure_admin_queue();
        init_ctx.set_entry_size();
        init_ctx.enable_controller()?;
        init_ctx.identify_controller()?;

        let nsids = init_ctx.identify_ns_list()?;
        if nsids.is_empty() {
            return Err(NvmeDeviceError::NoNamespace);
        }

        // TODO: Support exposing multiple namespaces per controller instead of only the first one.
        let namespace = init_ctx.identify_ns(nsids[0])?;

        let io_msix_vectors = init_ctx.create_io_queues()?;
        let device = NvmeDeviceInner {
            submission_queues: init_ctx.submission_queues.map(SpinLock::new),
            completion_queues: init_ctx.completion_queues.map(SpinLock::new),
            completion_wait_queues: [WaitQueue::new(), WaitQueue::new()],
            transport: NvmePciTransportLock::new(init_ctx.transport),
            namespace,
            dstrd: init_ctx.dstrd,
            max_io_bytes: init_ctx.max_io_bytes,
            stats: NvmeStats::new(),
        };
        Ok((device, io_msix_vectors))
    }

    /// Registers MSI-X handlers that wake completion wait queues for admin and I/O queues.
    fn setup_msix_handlers(
        &self,
        block_device: &Arc<NvmeBlockDevice>,
        io_msix_vectors: IoMsixVectors,
    ) {
        let mut transport = self.transport.lock();
        let msix_manager = transport.msix_manager_mut();

        // Admin queue interrupt (vector 0)
        let (_admin_vec, admin_irq) = msix_manager.admin_irq();
        let device_weak = Arc::downgrade(block_device);
        admin_irq.on_active(move |_| {
            if let Some(block_device) = device_weak.upgrade() {
                block_device.device.completion_wait_queues[0].wake_all();
            }
        });

        // I/O queues
        for io_qid in 1..QUEUE_NUM {
            let io_irq = msix_manager
                .irq_for_vector_mut(io_msix_vectors.0[io_qid - 1])
                .unwrap();
            let device_weak = Arc::downgrade(block_device);
            io_irq.on_active(move |_| {
                if let Some(block_device) = device_weak.upgrade() {
                    block_device.device.handle_io_completions();
                }
            });
        }
    }

    fn read(&self, request: BioRequest) {
        self.submit_request(request, IoOp::Read);
    }

    fn write(&self, request: BioRequest) {
        self.submit_request(request, IoOp::Write);
    }

    fn flush(&self, request: BioRequest) {
        let entry = nvme_cmd::io_flush(self.namespace.id);
        let inflight = Arc::new(InFlightRequest {
            bio_request: SpinLock::new(Some(request)),
            remaining: AtomicUsize::new(1),
            failed: AtomicBool::new(false),
        });
        self.submit_io_commands(&inflight, vec![(entry, None)]);
    }

    /// Builds and submits all NVMe commands for `bio_request` without waiting for completion.
    fn submit_request(&self, bio_request: BioRequest, io_op: IoOp) {
        let mut prepared = Vec::new();
        let mut build_failed = false;
        let mut lba = bio_request.sid_range().start.to_raw();

        for bio in bio_request.bios() {
            for segment in bio.segments() {
                let dma_slice = segment.inner_dma_slice();
                // `BioSegment` should guarantee that the segment's address and the size is
                // aligned to sectors.
                debug_assert!(dma_slice.daddr().is_multiple_of(SECTOR_SIZE));
                debug_assert!(dma_slice.size().is_multiple_of(SECTOR_SIZE));

                let mut dma_addr = dma_slice.daddr() as u64;
                let mut remaining_bytes = dma_slice.size();

                while let Some(remaining) = NonZeroUsize::new(remaining_bytes) {
                    let chunk_bytes = remaining.min(self.max_io_bytes);
                    debug_assert!(chunk_bytes.get().is_multiple_of(SECTOR_SIZE));

                    match self.build_io_command(lba, dma_addr, chunk_bytes, io_op) {
                        Ok(cmd) => prepared.push(cmd),
                        Err(_) => {
                            build_failed = true;
                        }
                    }

                    let chunk_sectors = (chunk_bytes.get() / SECTOR_SIZE) as u64;
                    lba += chunk_sectors;
                    dma_addr += chunk_bytes.get() as u64;
                    remaining_bytes -= chunk_bytes.get();
                }
            }
        }

        if prepared.is_empty() {
            let status = if build_failed {
                BioStatus::IoError
            } else {
                BioStatus::Complete
            };
            bio_request.into_bios().for_each(|bio| {
                bio.complete(status);
            });
            return;
        }

        let request = Arc::new(InFlightRequest {
            bio_request: SpinLock::new(Some(bio_request)),
            remaining: AtomicUsize::new(prepared.len()),
            failed: AtomicBool::new(build_failed),
        });

        let commands = prepared
            .into_iter()
            .map(|cmd| (cmd.entry, Some(cmd.prp)))
            .collect();
        self.submit_io_commands(&request, commands);
    }

    fn build_io_command(
        &self,
        lba: u64,
        dma_addr: u64,
        length: NonZeroUsize,
        io_op: IoOp,
    ) -> Result<BuiltIoCommand, NvmeDeviceError> {
        let nsid = self.namespace.id;
        let prp = PrpPointers::build_prp(dma_addr, length)?;
        let nlb = (length.get() / SECTOR_SIZE - 1) as u16;
        let entry = match io_op {
            IoOp::Read => nvme_cmd::io_read(nsid, lba, nlb, prp.prp1(), prp.prp2()),
            IoOp::Write => nvme_cmd::io_write(nsid, lba, nlb, prp.prp1(), prp.prp2()),
        };
        Ok(BuiltIoCommand { entry, prp })
    }

    /// Enqueues commands for a shared [`InFlightRequest`], then rings the SQ doorbell.
    ///
    /// Each command is an SQE plus optional PRP pages (`None` for commands like flush).
    fn submit_io_commands(
        &self,
        request: &Arc<InFlightRequest>,
        mut commands: Vec<(NvmeCommand, Option<PrpPointers>)>,
    ) {
        while !commands.is_empty() {
            let mut sq = self.lock_sq(IO_QID);
            let n = sq.free_slots().min(commands.len());
            if n == 0 {
                drop(sq);
                self.wait_for_sq_slot();
                continue;
            }

            let rest = commands.split_off(n);
            let batch = core::mem::replace(&mut commands, rest);

            if batch.len() > 1 {
                info!(
                    "Submitting a batch of {} I/O commands (queue depth in use)",
                    batch.len()
                );
            }

            let n_submitted = batch.len();
            sq.submit_with_items(batch.into_iter().map(|(entry, prp)| {
                (
                    entry,
                    SubmittedRequest {
                        request: request.clone(),
                        _prp: prp,
                    },
                )
            }))
            .expect("SQ `free_slots` indicated space for this `submit_with_items`");
            for _ in 0..n_submitted {
                self.stats.increment_submitted();
            }
        }
    }

    /// Waits until the I/O SQ has at least one free slot, polling completions while blocked.
    fn wait_for_sq_slot(&self) {
        let wait_queue = &self.completion_wait_queues[IO_QID];
        wait_queue.wait_until(|| self.handle_io_completions().then_some(()));
    }

    /// Drains ready I/O completions and finishes any requests whose commands have all completed.
    ///
    /// Each CQE is matched by CID to an entry in the I/O submission queue. Callers may invoke
    /// this from the MSI-X handler or from a waiting submitter when the SQ is full.
    ///
    /// Returns whether the I/O SQ has at least one free slot after processing.
    fn handle_io_completions(&self) -> bool {
        let completions = self.lock_cq(IO_QID).complete();

        let mut finished = Vec::new();
        let has_free_slot = {
            let mut sq = self.submission_queues[IO_QID].lock();
            if let Some(last) = completions.last() {
                sq.set_sq_head(last.sq_head());
            }

            for completion in &completions {
                let cid = completion.cid();
                let Some(submitted_request) = sq.take_item(cid) else {
                    warn!(
                        "ignoring unexpected NVMe completion: CID {} on QID {} has no submitted request",
                        cid, IO_QID
                    );
                    continue;
                };

                self.stats.increment_completed();

                if completion.has_error() {
                    submitted_request
                        .request
                        .failed
                        .store(true, Ordering::Relaxed);
                }

                if submitted_request
                    .request
                    .remaining
                    .fetch_sub(1, Ordering::Release)
                    == 1
                {
                    finished.push(submitted_request.request);
                }
            }

            sq.free_slots() > 0
        };

        for request in finished {
            request.finish();
        }

        if !completions.is_empty() {
            self.completion_wait_queues[IO_QID].wake_all();
        }

        has_free_slot
    }

    fn lock_sq(
        &self,
        qid: usize,
    ) -> NvmeSubmissionQueueAccess<
        '_,
        SpinLockGuard<'_, NvmeSubmissionQueue<SubmittedRequest>, LocalIrqDisabled>,
    > {
        NvmeSubmissionQueueAccess::new(
            qid as u16,
            self.dstrd,
            self.submission_queues[qid].lock(),
            self.transport.dbregs(),
        )
    }

    fn lock_cq(
        &self,
        qid: usize,
    ) -> NvmeCompletionQueueAccess<'_, SpinLockGuard<'_, NvmeCompletionQueue, LocalIrqDisabled>>
    {
        NvmeCompletionQueueAccess::new(
            qid as u16,
            self.dstrd,
            self.completion_queues[qid].lock(),
            self.transport.dbregs(),
        )
    }
}

struct InitContext {
    submission_queues: [NvmeSubmissionQueue<SubmittedRequest>; QUEUE_NUM],
    completion_queues: [NvmeCompletionQueue; QUEUE_NUM],
    transport: NvmePciTransport,
    dstrd: u16,
    cc_mps_value: u32,
    controller_ready_timeout: Duration,
    max_io_bytes: NonZeroUsize,
}

struct IoMsixVectors([u16; QUEUE_NUM - 1]);

impl InitContext {
    /// Controller Configuration Enable bit.
    const CC_ENABLE: u32 = 0x1;
    /// Controller Configuration I/O Submission Queue Entry Size shift.
    const IOSQES_SHIFT: u32 = 16;
    /// Controller Configuration I/O Submission Queue Entry Size value.
    const IOSQES_VALUE: u32 = 6;
    /// Controller Configuration I/O Completion Queue Entry Size shift.
    const IOCQES_SHIFT: u32 = 20;
    /// Controller Configuration I/O Completion Queue Entry Size value.
    const IOCQES_VALUE: u32 = 4;
    /// Controller Configuration memory page size shift.
    const MPS_SHIFT: u32 = 7;
    /// Controller Configuration command set selection shift.
    const CSS_SHIFT: u32 = 4;
    /// Controller Configuration arbitration mechanism shift.
    const AMS_SHIFT: u32 = 11;
    /// Controller Configuration memory page size mask.
    const MPS_MASK: u32 = 0b1111 << Self::MPS_SHIFT;
    /// Controller Configuration command set selection mask.
    const CSS_MASK: u32 = 0b111 << Self::CSS_SHIFT;
    /// Controller Configuration arbitration mechanism mask.
    const AMS_MASK: u32 = 0b111 << Self::AMS_SHIFT;
    /// Controller Configuration IOSQES field mask.
    const IOSQES_MASK: u32 = 0b1111 << Self::IOSQES_SHIFT;
    /// Controller Configuration IOCQES field mask.
    const IOCQES_MASK: u32 = 0b1111 << Self::IOCQES_SHIFT;
    /// Controller Status Ready bit.
    const CSTS_RDY: u32 = 0x1;
    /// Controller Fatal Status bit.
    const CSTS_CFS: u32 = 0x2;

    fn new(
        transport: NvmePciTransport,
        dstrd: u16,
        cc_mps_value: u32,
        controller_ready_timeout: Duration,
    ) -> Result<Self, NvmeDeviceError> {
        let sq0 = NvmeSubmissionQueue::<SubmittedRequest>::new()
            .ok_or(NvmeDeviceError::QueueAllocationFailed)?;
        let sq1 = NvmeSubmissionQueue::<SubmittedRequest>::new()
            .ok_or(NvmeDeviceError::QueueAllocationFailed)?;
        let cq0 = NvmeCompletionQueue::new().ok_or(NvmeDeviceError::QueueAllocationFailed)?;
        let cq1 = NvmeCompletionQueue::new().ok_or(NvmeDeviceError::QueueAllocationFailed)?;
        Ok(Self {
            submission_queues: [sq0, sq1],
            completion_queues: [cq0, cq1],
            transport,
            dstrd,
            cc_mps_value,
            controller_ready_timeout,
            max_io_bytes: MAX_BYTES_BY_NLB,
        })
    }

    fn reset_controller(&mut self) -> Result<(), NvmeDeviceError> {
        let mut cc = self.transport.regs().read32(NvmeRegs32::Cc);
        if (cc & Self::CC_ENABLE) != 0 {
            self.wait_controller_ready(true)?;
            cc &= !Self::CC_ENABLE;
            self.transport.regs().write32(NvmeRegs32::Cc, cc);
        }
        self.wait_controller_ready(false)
    }

    fn configure_admin_queue(&mut self) {
        let acq = &self.completion_queues[0];
        let asq = &self.submission_queues[0];

        self.transport.regs().write32(
            NvmeRegs32::Aqa,
            (((QUEUE_DEPTH - 1) as u32) << 16) | ((QUEUE_DEPTH - 1) as u32),
        );
        self.transport
            .regs()
            .write64(NvmeRegs64::Asq, asq.sq_daddr() as u64);
        self.transport
            .regs()
            .write64(NvmeRegs64::Acq, acq.cq_daddr() as u64);
    }

    fn set_entry_size(&mut self) {
        let mut cc = self.transport.regs().read32(NvmeRegs32::Cc);
        cc &= !(Self::MPS_MASK
            | Self::CSS_MASK
            | Self::AMS_MASK
            | Self::IOSQES_MASK
            | Self::IOCQES_MASK);
        cc |= self.cc_mps_value << Self::MPS_SHIFT;
        cc |= Self::IOSQES_VALUE << Self::IOSQES_SHIFT;
        cc |= Self::IOCQES_VALUE << Self::IOCQES_SHIFT;
        self.transport.regs().write32(NvmeRegs32::Cc, cc);
    }

    fn enable_controller(&mut self) -> Result<(), NvmeDeviceError> {
        let mut cc = self.transport.regs().read32(NvmeRegs32::Cc);
        cc |= Self::CC_ENABLE;
        self.transport.regs().write32(NvmeRegs32::Cc, cc);
        self.wait_controller_ready(true)
    }

    fn identify_controller(&mut self) -> Result<(), NvmeDeviceError> {
        let stream =
            DmaStream::alloc(1, false).map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
        let data: SafePtr<IdentifyControllerData, DmaStream> = SafePtr::new(stream, 0);

        let entry = nvme_cmd::identify_controller(data.daddr());
        self.submit_and_wait_polling(ADMIN_QID, entry)?;

        let result = data.read().unwrap();

        let serial = bytes_to_cstr_string(&result.serial);
        let model = bytes_to_cstr_string(&result.model);
        let firmware = bytes_to_cstr_string(&result.firmware);

        self.max_io_bytes = max_io_bytes_from_mdts(result.mdts);

        info!(
            "Controller identified - Serial: {}, Model: {}, Firmware: {}, max I/O={} bytes (MDTS={})",
            serial,
            model,
            firmware,
            self.max_io_bytes.get(),
            result.mdts
        );
        Ok(())
    }

    fn identify_ns_list(&mut self) -> Result<Vec<u32>, NvmeDeviceError> {
        let stream =
            DmaStream::alloc(1, false).map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
        let data: SafePtr<IdentifyNamespaceListData, DmaStream> = SafePtr::new(stream, 0);

        let entry = nvme_cmd::identify_namespace_list(data.daddr(), 0);
        self.submit_and_wait_polling(ADMIN_QID, entry)?;

        let result = data.read().unwrap();

        let nsids = result
            .nsids
            .iter()
            .copied()
            .take_while(|&nsid| nsid != 0)
            .collect();
        Ok(nsids)
    }

    fn identify_ns(&mut self, nsid: u32) -> Result<NvmeNamespace, NvmeDeviceError> {
        let stream =
            DmaStream::alloc(1, false).map_err(|_| NvmeDeviceError::DmaAllocationFailed)?;
        let data: SafePtr<IdentifyNamespaceData, DmaStream> = SafePtr::new(stream, 0);

        let entry = nvme_cmd::identify_namespace(data.daddr(), nsid);
        self.submit_and_wait_polling(ADMIN_QID, entry)?;

        let result = data.read().unwrap();

        // Parse the active LBA format to obtain the logical block size.
        // FLBAS bits[3:0] select the current format entry in `lbaf[]`.
        // This driver currently supports only the lower 16 LBAF entries.
        // LBADS (bits[23:16] of that entry) is the base-2 exponent of the block size.
        if result.flbas & 0x60 != 0 {
            error!(
                "Namespace {}: FLBAS uses an extended active format index (unsupported)",
                nsid
            );
            return Err(NvmeDeviceError::InvalidControllerConfig);
        }
        let fmt_idx = (result.flbas & 0x0f) as usize;
        let lbads = (result.lbaf[fmt_idx] >> 16) & 0xff;
        let reported_lba_size = 1u64 << lbads;
        if reported_lba_size != LBA_SIZE as u64 {
            error!(
                "Namespace {}: LBA size is {} bytes (LBADS={}), but driver requires {}",
                nsid, reported_lba_size, lbads, LBA_SIZE
            );
            return Err(NvmeDeviceError::InvalidControllerConfig);
        }

        info!(
            "Namespace {}: NSZE={}, LBA size={} bytes",
            nsid, result.nsze, LBA_SIZE
        );
        Ok(NvmeNamespace {
            id: nsid,
            nsze: result.nsze,
        })
    }

    fn create_io_queues(&mut self) -> Result<IoMsixVectors, NvmeDeviceError> {
        // Pre-allocate MSI-X vectors for I/O queues
        let io_msix_vectors = {
            let mut io_msix_vectors = [0; QUEUE_NUM - 1];
            let msix_manager = self.transport.msix_manager_mut();
            for io_qid in 1..QUEUE_NUM {
                let (vector, _io_irq) = msix_manager
                    .alloc_io_queue_irq()
                    .ok_or(NvmeDeviceError::MsixAllocationFailed)?;
                io_msix_vectors[io_qid - 1] = vector;
            }
            io_msix_vectors
        };

        for io_qid in 1..QUEUE_NUM {
            let cptr = self.completion_queues[io_qid].cq_daddr();

            let msix_vector = io_msix_vectors[io_qid - 1];

            let entry = nvme_cmd::create_io_completion_queue(
                io_qid as u16,
                cptr,
                (QUEUE_DEPTH - 1) as u16,
                Some(msix_vector),
            );
            self.submit_and_wait_polling(ADMIN_QID, entry)?;

            let sptr = self.submission_queues[io_qid].sq_daddr();

            let entry = nvme_cmd::create_io_submission_queue(
                io_qid as u16,
                sptr,
                (QUEUE_DEPTH - 1) as u16,
                io_qid as u16,
            );
            self.submit_and_wait_polling(ADMIN_QID, entry)?;
        }

        Ok(IoMsixVectors(io_msix_vectors))
    }

    fn submit_and_wait_polling(
        &mut self,
        qid: usize,
        entry: NvmeCommand,
    ) -> Result<(), NvmeDeviceError> {
        let expected_cid = self
            .sq_mut(qid)
            .submit(entry)
            .ok_or(NvmeDeviceError::SubmissionQueueFull)?;

        loop {
            let completions = self.cq_mut(qid).complete();
            if completions.is_empty() {
                spin_loop();
                continue;
            }

            if let Some(last) = completions.last() {
                self.submission_queues[qid].update_sq_head(last);
            }

            let mut matched = None;
            for cqe in &completions {
                if cqe.cid() != expected_cid {
                    debug!(
                        "Ignore unexpected completion in polling path: expected CID {}, got {} on QID {}",
                        expected_cid,
                        cqe.cid(),
                        qid
                    );
                    continue;
                }
                matched = Some(cqe);
            }

            let Some(cqe) = matched else {
                continue;
            };
            if cqe.has_error() {
                return Err(NvmeDeviceError::CommandFailed);
            }

            return Ok(());
        }
    }

    fn wait_controller_ready(&mut self, expected_ready: bool) -> Result<(), NvmeDeviceError> {
        let start = Jiffies::elapsed().as_duration();
        let deadline = start
            .checked_add(self.controller_ready_timeout)
            .unwrap_or(Duration::MAX);

        loop {
            let csts = self.transport.regs().read32(NvmeRegs32::Csts);
            if (csts & Self::CSTS_CFS) != 0 {
                error!(
                    "Controller reports fatal status during reset/enable: CSTS={:#x}",
                    csts
                );
                return Err(NvmeDeviceError::ControllerEnableTimeout);
            }
            let ready = (csts & Self::CSTS_RDY) != 0;
            if ready == expected_ready {
                return Ok(());
            }
            if Jiffies::elapsed().as_duration() >= deadline {
                error!(
                    "Controller ready transition timed out: expected RDY={}, CSTS={:#x}",
                    expected_ready as u8, csts
                );
                return Err(NvmeDeviceError::ControllerEnableTimeout);
            }
            spin_loop();
        }
    }

    fn sq_mut(
        &mut self,
        qid: usize,
    ) -> NvmeSubmissionQueueAccess<'_, &mut NvmeSubmissionQueue<SubmittedRequest>> {
        NvmeSubmissionQueueAccess::new(
            qid as u16,
            self.dstrd,
            &mut self.submission_queues[qid],
            self.transport.dbregs(),
        )
    }

    fn cq_mut(&mut self, qid: usize) -> NvmeCompletionQueueAccess<'_, &mut NvmeCompletionQueue> {
        NvmeCompletionQueueAccess::new(
            qid as u16,
            self.dstrd,
            &mut self.completion_queues[qid],
            self.transport.dbregs(),
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct IdentifyControllerData {
    _reserved: [u8; 4],
    serial: [u8; 20],
    model: [u8; 40],
    firmware: [u8; 8],
    _bytes_72_76: [u8; 5],
    mdts: u8,
    _rest: [u8; 55],
}

#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct IdentifyNamespaceListData {
    nsids: [u32; MAX_NS_NUM],
}

/// Identify Namespace data structure returned for CNS 00h
/// (NVM Command Set Specification Figure 114).
///
/// Only the fields needed to determine the active LBA format are captured here;
/// the remaining bytes of the 4096-byte response are not used.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct IdentifyNamespaceData {
    /// NSZE: total number of logical blocks.
    nsze: u64,
    /// NCAP: namespace capacity in logical blocks.
    _ncap: u64,
    /// NUSE: namespace utilization in logical blocks.
    _nuse: u64,
    /// NSFEAT (byte 24).
    _nsfeat: u8,
    /// NLBAF: number of LBA formats supported minus one (byte 25).
    _nlbaf: u8,
    /// FLBAS: formatted LBA size; bits[3:0] index into `lbaf[]` (byte 26).
    flbas: u8,
    /// RESERVED: bytes 27–127.
    _reserved: [u8; 101],
    /// LBAF[0..15]: LBA format support descriptors (bytes 128–191).
    ///
    /// Each entry is a u32: bits[23:16] = LBADS (LBA data-size exponent,
    /// so actual size = 2^LBADS bytes).
    lbaf: [u32; 16],
}

fn bytes_to_cstr_string(bytes: &[u8]) -> String {
    if let Ok(cstr) = CStr::from_bytes_until_nul(bytes) {
        let s = cstr.to_string_lossy();
        s.trim_end().to_owned()
    } else {
        String::new()
    }
}

/// Maximum bytes transferable in one NVMe command.
///
/// `NLB` is zero-based in a `u16`, so one command covers at most 65536 LBAs.
const MAX_BYTES_BY_NLB: NonZeroUsize =
    NonZeroUsize::new((u16::MAX as usize + 1) * SECTOR_SIZE).unwrap();

/// Converts Identify Controller MDTS into an effective per-command byte limit.
///
/// MDTS `0` means no device-side limit; we still cap at [`MAX_BYTES_BY_NLB`].
fn max_io_bytes_from_mdts(mdts: u8) -> NonZeroUsize {
    if mdts == 0 {
        return MAX_BYTES_BY_NLB;
    }

    const PAGE_SIZE_NONZERO: NonZeroUsize = NonZeroUsize::new(PAGE_SIZE).unwrap();

    let Some(pages) = 1usize
        .checked_shl(u32::from(mdts))
        .and_then(NonZeroUsize::new)
    else {
        warn!(
            "Invalid MDTS {}, falling back to {}-byte I/O limit",
            mdts,
            MAX_BYTES_BY_NLB.get()
        );
        return MAX_BYTES_BY_NLB;
    };
    let Some(bytes) = pages.checked_mul(PAGE_SIZE_NONZERO) else {
        return MAX_BYTES_BY_NLB;
    };

    bytes.min(MAX_BYTES_BY_NLB)
}

#[cfg(ktest)]
mod test {
    use alloc::{sync::Arc, vec, vec::Vec};

    use aster_block::{
        BLOCK_SIZE,
        bio::{Bio, BioDirection, BioSegment},
        id::{Bid, Sid},
    };
    use io_util::batch::IoBatch;
    use ostd::{
        info,
        mm::{FrameAllocOptions, VmIo, VmReader, io::util::HasVmReaderWriter},
        prelude::ktest,
    };

    use super::{BioType, IoOp, NvmeBlockDevice};
    use crate::nvme_init;

    const TEST_CHAR: u8 = b'B';
    const TEST_BUF_LENGTH: usize = 8192;

    #[ktest]
    fn initialize() {
        ensure_initialized();
    }

    fn ensure_initialized() {
        if aster_block::collect_all().is_empty() {
            component::init_all(
                component::InitStage::Bootstrap,
                component::parse_metadata!(),
            )
            .unwrap();

            nvme_init().expect("`nvme_init` returned an error");
        }
    }

    #[ktest]
    fn write_then_read() {
        ensure_initialized();

        let device = match aster_block::collect_all()
            .into_iter()
            .find(|d| d.name() == "nvme0n1")
        {
            Some(device) => device,
            None => {
                info!("Skip nvme ktest: NVMe device not found");
                return;
            }
        };
        let device_arc = Arc::clone(&device);

        let nvme_block_device = device_arc
            .downcast_ref::<NvmeBlockDevice>()
            .expect("Failed to downcast device");

        let mut write_batch = IoBatch::with_capacity(1);
        create_and_submit_bio_request(
            nvme_block_device,
            &mut write_batch,
            IoOp::Write,
            TEST_BUF_LENGTH,
            TEST_CHAR,
        );
        nvme_block_device.handle_requests();
        write_batch.wait_all().unwrap();

        let mut read_batch = IoBatch::with_capacity(1);
        let read_bio_segment = create_and_submit_bio_request(
            nvme_block_device,
            &mut read_batch,
            IoOp::Read,
            TEST_BUF_LENGTH,
            TEST_CHAR,
        );
        nvme_block_device.handle_requests();
        read_batch.wait_all().unwrap();

        let mut read_buf = [0u8; TEST_BUF_LENGTH];
        read_bio_segment
            .inner_dma_slice()
            .read_bytes(0, &mut read_buf)
            .unwrap();
        assert!(read_buf.iter().all(|&x| x == TEST_CHAR));
    }

    fn create_and_submit_bio_request(
        device: &NvmeBlockDevice,
        io_batch: &mut IoBatch,
        req_type: IoOp,
        buf_len: usize,
        val: u8,
    ) -> BioSegment {
        let buf_nblocks = buf_len / BLOCK_SIZE;
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(buf_nblocks)
            .unwrap();

        if matches!(req_type, IoOp::Write) {
            let mut writer = segment.writer();
            let fill_buf = [val; BLOCK_SIZE];
            for _ in 0..buf_nblocks {
                let mut reader = VmReader::from(fill_buf.as_slice());
                writer.write(&mut reader);
            }
        }

        let (bio_type, direction) = match req_type {
            IoOp::Write => (BioType::Write, BioDirection::ToDevice),
            IoOp::Read => (BioType::Read, BioDirection::FromDevice),
        };
        let bio_segment = BioSegment::new_from_segment(segment.into(), direction);

        let bio = Bio::new(
            bio_type,
            Sid::from(Bid::from_offset(0)),
            vec![bio_segment.clone()],
            None,
        );
        bio.submit(device, io_batch).unwrap();

        bio_segment
    }

    /// Submits one bio built from many single-page segments so the driver must issue
    /// multiple NVMe commands for that bio.
    #[ktest]
    fn multi_segment_bio_uses_queue_depth() {
        ensure_initialized();

        let device = match aster_block::collect_all()
            .into_iter()
            .find(|d| d.name() == "nvme0n1")
        {
            Some(device) => device,
            None => {
                info!("Skip nvme ktest: NVMe device not found");
                return;
            }
        };
        let nvme_block_device = device
            .downcast_ref::<NvmeBlockDevice>()
            .expect("Failed to downcast device");

        const NR_SEGMENTS: usize = 16;
        nvme_block_device.reset_io_stats();

        let mut write_batch = IoBatch::with_capacity(1);
        create_and_submit_multi_segment_bio(
            nvme_block_device,
            &mut write_batch,
            IoOp::Write,
            NR_SEGMENTS,
            TEST_CHAR,
        );
        nvme_block_device.handle_requests();
        write_batch.wait_all().unwrap();

        let peak = nvme_block_device.max_in_flight_commands();

        assert!(
            peak >= NR_SEGMENTS as u64,
            "expected at least {} in-flight commands at once, got peak {}",
            NR_SEGMENTS,
            peak
        );
    }

    fn create_and_submit_multi_segment_bio(
        device: &NvmeBlockDevice,
        io_batch: &mut IoBatch,
        req_type: IoOp,
        nr_segments: usize,
        val: u8,
    ) {
        let mut segments = Vec::with_capacity(nr_segments);
        for _ in 0..nr_segments {
            let segment = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_segment(1)
                .unwrap();

            if matches!(req_type, IoOp::Write) {
                let mut writer = segment.writer();
                let fill_buf = [val; BLOCK_SIZE];
                let mut reader = VmReader::from(fill_buf.as_slice());
                writer.write(&mut reader);
            }

            segments.push(segment);
        }

        let (bio_type, direction) = match req_type {
            IoOp::Write => (BioType::Write, BioDirection::ToDevice),
            IoOp::Read => (BioType::Read, BioDirection::FromDevice),
        };
        let bio_segments: Vec<_> = segments
            .into_iter()
            .map(|segment| BioSegment::new_from_segment(segment.into(), direction))
            .collect();

        let bio = Bio::new(bio_type, Sid::from(Bid::from_offset(0)), bio_segments, None);
        bio.submit(device, io_batch).unwrap();
    }
}
