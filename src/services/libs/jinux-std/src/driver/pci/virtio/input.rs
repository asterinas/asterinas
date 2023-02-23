use core::any::Any;
use core::sync::atomic::AtomicBool;

use alloc::collections::BTreeMap;
use alloc::{string::String, sync::Arc, vec::Vec};
use jinux_frame::{offset_of, TrapFrame};
use jinux_pci::{msix::MSIX, PCIDevice};
use jinux_util::frame_ptr::InFramePtr;
use jinux_virtio::device::input::device::InputProp;
use jinux_virtio::VitrioPciCommonCfg;
use jinux_virtio::{
    device::input::{device::InputDevice, InputConfigSelect},
    PCIVirtioDevice,
};
use lazy_static::lazy_static;
use log::{debug, info};
use spin::Mutex;
use virtio_input_decoder::{DecodeType, Decoder};

pub trait INPUTDevice: Send + Sync + Any {
    fn handle_irq(&self) -> Option<()>;
}

lazy_static! {
    pub static ref KEYBOARD_EVENT: Mutex<Vec<DecodeType>> = Mutex::new(Vec::new());
    pub static ref MOUSE_EVENT: Mutex<Vec<DecodeType>> = Mutex::new(Vec::new());
    static ref KEYBOARD_CALLBACKS: Mutex<Vec<Arc<dyn Fn() + Send + Sync + 'static>>> =
        Mutex::new(Vec::new());
    static ref INPUT_DEVICE_LIST: Mutex<Vec<Arc<VirtioInputDevice>>> =
        Mutex::new(Vec::with_capacity(2));
    static ref INPUT_DEVICE_IRQ_HASH: Mutex<BTreeMap<u16, usize>> = Mutex::new(BTreeMap::new());
}

pub struct VirtioInputDevice {
    input_device: InputDevice,
    common_cfg: InFramePtr<VitrioPciCommonCfg>,
    msix: Mutex<MSIX>,
    is_keyboard: AtomicBool,
}

impl VirtioInputDevice {
    fn new(virtio_device: PCIVirtioDevice, id: usize) -> Self {
        let input_device = match virtio_device.device {
            jinux_virtio::device::VirtioDevice::Input(dev) => dev,
            _ => {
                panic!("Error when creating new input device, the input device is other type of virtio device");
            }
        };

        Self {
            input_device,
            common_cfg: virtio_device.common_cfg,
            msix: Mutex::new(virtio_device.msix),
            is_keyboard: AtomicBool::new(false),
        }
    }

    fn register_interrupts(&self, id: usize) {
        fn handle_input(frame: &TrapFrame) {
            info!("in handle input");
            let id = *INPUT_DEVICE_IRQ_HASH
                .lock()
                .get(&(frame.id as u16))
                .expect("wrong irq number in input device trap handler");
            INPUT_DEVICE_LIST
                .lock()
                .get(id)
                .as_ref()
                .unwrap()
                .handle_irq();
        }
        fn config_space_change(frame: &TrapFrame) {}

        let config_msix_vector =
            self.common_cfg
                .read_at(offset_of!(VitrioPciCommonCfg, config_msix_vector)) as usize;
        let mut device_hash_lock = INPUT_DEVICE_IRQ_HASH.lock();
        let mut msix = self.msix.lock();
        for i in 0..msix.table_size as usize {
            if i == config_msix_vector {
                continue;
            }
            device_hash_lock.insert(msix.table.get(i).unwrap().irq_handle.num() as u16, id);
        }
        drop(device_hash_lock);
        for i in 0..msix.table_size as usize {
            let msix = msix.table.get_mut(i).unwrap();
            if !msix.irq_handle.is_empty() {
                panic!("function `register_queue_interrupt_functions` called more than one time");
            }
            if config_msix_vector == i {
                msix.irq_handle.on_active(config_space_change);
            } else {
                msix.irq_handle.on_active(handle_input);
            }
        }
    }

    fn print_device_information(&self) {
        let mut raw_name: [u8; 128] = [0; 128];
        self.input_device
            .query_config_select(InputConfigSelect::IdName, 0, &mut raw_name);
        let name = String::from_utf8(raw_name.to_vec()).unwrap();
        info!("input device name:{}", name);
        let mut prop: [u8; 128] = [0; 128];
        self.input_device
            .query_config_select(InputConfigSelect::PropBits, 0, &mut prop);

        let input_prop = InputProp::from_bits(prop[0]).unwrap();
        debug!("input device prop:{:?}", input_prop);

        // if name.contains("Keyboard"){
        //     let mut raw_ev : [u8;128] = [0;128];
        //     let size = self.input_device.query_config_select(InputConfigSelect::EvBits, KEY, &mut raw_ev);
        //     info!("size:{}, raw_ev :{:x?}",size, raw_ev);

        // }else{
        //     let mut raw_ev : [u8;128] = [0;128];
        //     let size = self.input_device.query_config_select(InputConfigSelect::EvBits, REL, &mut raw_ev);
        //     info!("size:{}, raw_ev :{:x?}",size, raw_ev);
        // }
        self.is_keyboard.store(
            name.contains("Keyboard"),
            core::sync::atomic::Ordering::Relaxed,
        );
    }

    #[inline]
    pub fn is_keyboard(&self) -> bool {
        self.is_keyboard.load(core::sync::atomic::Ordering::Relaxed)
    }
}

impl INPUTDevice for VirtioInputDevice {
    fn handle_irq(&self) -> Option<()> {
        let input = &self.input_device;
        // one interrupt may contains serval input, so it should loop
        loop {
            let event = input.pop_pending_event()?;
            let dtype = match Decoder::decode(
                event.event_type as usize,
                event.code as usize,
                event.value as usize,
            ) {
                Ok(dtype) => dtype,
                Err(_) => return Some(()),
            };
            if self.is_keyboard() {
                let mut lock = KEYBOARD_EVENT.lock();
                lock.push(dtype);
                drop(lock);
                let lock = KEYBOARD_CALLBACKS.lock();
                for callback in lock.iter() {
                    callback.call(());
                }
            } else {
                let mut lock = MOUSE_EVENT.lock();
                lock.push(dtype);
            }
            match dtype {
                virtio_input_decoder::DecodeType::Key(key, r#type) => {
                    info!("{:?} {:?}", key, r#type);
                }
                virtio_input_decoder::DecodeType::Mouse(mouse) => info!("{:?}", mouse),
            }
        }
    }
}

pub fn init(pci_device: Arc<PCIDevice>) {
    let mut lock = INPUT_DEVICE_LIST.lock();
    let id = lock.len();
    let dev = Arc::new(VirtioInputDevice::new(PCIVirtioDevice::new(pci_device), id));
    lock.push(dev.clone());
    dev.register_interrupts(id);
    drop(lock);
    dev.print_device_information();
}

pub fn register_keyboard_callback(callback: Arc<dyn Fn() + 'static + Send + Sync>) {
    KEYBOARD_CALLBACKS.lock().push(callback);
}
