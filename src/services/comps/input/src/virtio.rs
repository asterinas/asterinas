//! Input device based on Virtio

use alloc::{string::String, sync::Arc, vec::Vec};
use jinux_frame::{offset_of, TrapFrame};
use jinux_pci::msix::MSIX;
use jinux_util::frame_ptr::InFramePtr;
use jinux_virtio::device::input::device::InputProp;
use jinux_virtio::VitrioPciCommonCfg;
use jinux_virtio::{
    device::input::{device::InputDevice, InputConfigSelect},
    PCIVirtioDevice,
};
use log::{debug, info};
use spin::Mutex;
use virtio_input_decoder::{DecodeType, Decoder};

use crate::INPUTDevice;
pub struct VirtioInputDevice {
    input_device: InputDevice,
    common_cfg: InFramePtr<VitrioPciCommonCfg>,
    msix: Mutex<MSIX>,
    name: String,
    callbacks: Mutex<Vec<Arc<dyn Fn(DecodeType) + Send + Sync + 'static>>>,
}

impl VirtioInputDevice {
    /// Create a new Virtio Input Device, return value contains the irq number it will use
    pub(crate) fn new(virtio_device: PCIVirtioDevice) -> (Self, u8) {
        let input_device = match virtio_device.device {
            jinux_virtio::device::VirtioDevice::Input(dev) => dev,
            _ => {
                panic!("Error when creating new input device, the input device is other type of virtio device");
            }
        };
        let mut raw_name: [u8; 128] = [0; 128];
        input_device.query_config_select(InputConfigSelect::IdName, 0, &mut raw_name);
        let name = String::from_utf8(raw_name.to_vec()).unwrap();
        info!("input device name:{}", name);

        let mut prop: [u8; 128] = [0; 128];
        input_device.query_config_select(InputConfigSelect::PropBits, 0, &mut prop);

        let input_prop = InputProp::from_bits(prop[0]).unwrap();
        debug!("input device prop:{:?}", input_prop);

        fn handle_input(frame: &TrapFrame) {
            debug!("in handle input");
            let input_component = crate::INPUT_COMPONENT.get().unwrap();
            input_component.call(frame.id as u8);
        }
        fn config_space_change(_: &TrapFrame) {
            debug!("input device config space change");
        }

        let common_cfg = virtio_device.common_cfg;
        let mut msix = virtio_device.msix;

        let config_msix_vector =
            common_cfg.read_at(offset_of!(VitrioPciCommonCfg, config_msix_vector)) as usize;

        let mut event_irq_number = 0;
        for i in 0..msix.table_size as usize {
            let msix = msix.table.get_mut(i).unwrap();
            if !msix.irq_handle.is_empty() {
                panic!("msix already have irq functions");
            }
            if config_msix_vector == i {
                msix.irq_handle.on_active(config_space_change);
            } else {
                event_irq_number = msix.irq_handle.num();
                msix.irq_handle.on_active(handle_input);
            }
        }

        (
            Self {
                input_device,
                common_cfg,
                msix: Mutex::new(msix),
                name,
                callbacks: Mutex::new(Vec::new()),
            },
            event_irq_number,
        )
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
            let lock = self.callbacks.lock();
            for callback in lock.iter() {
                callback.call((dtype,));
            }
            match dtype {
                virtio_input_decoder::DecodeType::Key(key, r#type) => {
                    info!("{:?} {:?}", key, r#type);
                }
                virtio_input_decoder::DecodeType::Mouse(mouse) => info!("{:?}", mouse),
            }
        }
    }

    fn register_callbacks(&self, function: &'static (dyn Fn(DecodeType) + Send + Sync)) {
        self.callbacks.lock().push(Arc::new(function))
    }

    fn name(&self) -> &String {
        &self.name
    }
}
