use core::hint::spin_loop;

use alloc::collections::BTreeMap;
use pci::{CSpaceAccessMethod, PCIDevice};

use crate::{
    drivers::{
        msix::CapabilityMSIXData,
        pci::*,
        virtio::{block::*, queue::VirtQueue, *},
    },
    info,
    task::Task,
    zero, Error,
};

use super::BlockDevice;

pub struct VirtIOBlock {
    // virtio_blk: Cell<VirtIOBlk<'static, VirtioHal>>,
    common_cfg: &'static mut VitrioPciCommonCfg,
    dev_cfg: &'static mut VirtioBLKConfig,
    queue: Cell<VirtQueue>,
    tasks: BTreeMap<u16, Option<Task>>,
    irq_callback: IrqCallbackHandle,
}

impl BlockDevice for VirtIOBlock {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) -> Result<()> {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::In,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let queue = self.queue.get();
        queue
            .add(&[req.as_buf()], &[buf, resp.as_buf_mut()])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");
        match resp.status {
            RespStatus::Ok => Ok(()),
            _ => Err(Error::IoError),
        }
    }
    /// it is blocking now
    fn write_block(&self, block_id: usize, buf: &[u8]) -> Result<()> {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::Out,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let queue = self.queue.get();
        queue
            .add(&[req.as_buf(), buf], &[resp.as_buf_mut()])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");
        match resp.status {
            RespStatus::Ok => Ok(()),
            _ => Err(Error::IoError),
        }
    }
    fn handle_irq(&self) {
        info!("handle irq in block device!");
    }
}

impl VirtIOBlock {
    pub fn new(dev: PCIDevice) -> Self {
        fn handle_block_device(frame: TrapFrame) {
            BLOCK_DEVICE.get().handle_irq()
        }
        let (msix, common_cfg, dev_cfg, cap_offset, notify_off_multiplier);
        unsafe {
            (msix, common_cfg, dev_cfg, cap_offset, notify_off_multiplier) = Self::enable(dev.loc)
        };
        common_cfg.device_status = DeviceStatus::ACKNOWLEDGE.bits();
        common_cfg.device_status = DeviceStatus::DRIVER.bits();
        common_cfg.device_status = DeviceStatus::FEATURES_OK.bits();
        let queue = VirtQueue::new(common_cfg, 0, 16, cap_offset, notify_off_multiplier)
            .expect("error creating virtqueue");
        common_cfg.queue_enable = 1;
        common_cfg.device_status = DeviceStatus::DRIVER_OK.bits();
        let mut tasks = BTreeMap::new();
        let channels = queue.size();
        for i in 0..channels {
            tasks.insert(i, None);
        }
        let msix_entry = msix
            .table
            .get(common_cfg.queue_msix_vector as usize)
            .unwrap();
        // register interrupt
        let irq_number = msix_entry.allocate_irq;
        let irq;
        unsafe {
            irq = IrqLine::acquire(irq_number);
        }
        let blk = Self {
            common_cfg,
            dev_cfg,
            queue: Cell::new(queue),
            tasks: tasks,
            irq_callback: irq.on_active(handle_block_device),
        };
        blk
    }

    /// Enable the pci device and virtio MSIX
    /// need to activate the specific device
    /// return the msix, virtio pci common cfg, virtio block device config,
    /// the virtual address of cap.offset and notify_off_multiplier
    unsafe fn enable(
        loc: Location,
    ) -> (
        CapabilityMSIXData,
        &'static mut VitrioPciCommonCfg,
        &'static mut VirtioBLKConfig,
        usize,
        u32,
    ) {
        let ops = &PortOpsImpl;
        let am = CSpaceAccessMethod::IO;

        // 23 and lower are used, use 22-27
        static mut MSI_IRQ: u32 = 23;
        let mut cap_ptr = am.read8(ops, loc, PCI_CAP_PTR) as u16;
        let mut msix = zero();
        let mut init = false;
        let mut common_cfg = zero();
        let mut dev_cfg = zero();
        let mut notify_off_multiplier: u32 = 0;
        let mut cap_offset: usize = 0;
        while cap_ptr > 0 {
            let cap_vndr = am.read8(ops, loc, cap_ptr);
            match cap_vndr {
                9 => {
                    let cap = PciVirtioCapability::handle(loc, cap_ptr);
                    match cap.cfg {
                        CFGType::COMMON(x) => {
                            common_cfg = x;
                        }
                        CFGType::NOTIFY(x) => {
                            let bar = cap.bar;
                            let bar_address =
                                am.read32(ops, loc, PCI_BAR + bar as u16 * 4) & (!(0b1111));
                            cap_offset = mm::phys_to_virt((bar_address + cap.offset) as usize);
                            notify_off_multiplier = x;
                        }
                        CFGType::DEVICE(dev) => {
                            match dev {
                                VirtioDeviceCFG::Block(x) => dev_cfg = x,
                                _ => {
                                    panic!("wrong device while initalize virtio block device")
                                }
                            };
                        }
                        _ => {}
                    };
                }
                17 => {
                    msix = CapabilityMSIXData::handle(loc, cap_ptr);
                    init = true;
                }
                _ => panic!("unsupport capability, id:{}", cap_vndr),
            };
            cap_ptr = am.read8(ops, loc, cap_ptr + 1) as u16;
        }
        if !init {
            panic!("PCI Virtio Block Device initalize incomplete, not found msix");
        }
        common_cfg.queue_msix_vector = 0;
        (msix, common_cfg, dev_cfg, cap_offset, notify_off_multiplier)
    }
}
