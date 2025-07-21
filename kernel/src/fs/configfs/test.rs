// SPDX-License-Identifier: MPL-2.0

use alloc::{string::ToString, sync::Arc};
use core::{
    fmt::Debug,
    mem::size_of,
    sync::atomic::{AtomicU16, Ordering},
};

use aster_systree::{
    inherit_sys_branch_node, inherit_sys_leaf_node, BranchNodeFields, Error, NormalNodeFields,
    Result, SysAttrSet, SysAttrSetBuilder, SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
    Pod,
};
use spin::Once;

use crate::{
    fs::utils::{FileSystem, InodeMode, InodeType},
    time::clocks::init_for_ktest as time_init_for_ktest,
};

/// A simplified USB Gadget Subsystem.
///
/// This is a mock version to demonstrate the ConfigFS API.
#[derive(Debug)]
struct UsbGadgetSystem {
    fields: BranchNodeFields<UsbGadget, Self>,
}

#[inherit_methods(from = "self.fields")]
impl UsbGadgetSystem {
    fn new() -> Arc<Self> {
        let name = SysStr::from("usb_gadget");

        let attrs = SysAttrSet::new_empty();
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            UsbGadgetSystem { fields }
        })
    }

    fn add_child(&self, new_child: Arc<UsbGadget>) -> Result<()>;
}

inherit_sys_branch_node!(UsbGadgetSystem, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let gadget = UsbGadget::new(SysStr::from(name.to_string()));
        self.add_child(gadget.clone())?;
        Ok(gadget)
    }
});

/// A specific USB Gadget device.
///
/// Created by `mkdir [gadget_name]` (e.g., `g1`) in the `usb_gadget` directory.
#[derive(Debug)]
struct UsbGadget {
    fields: BranchNodeFields<UsbConfig, Self>,
    id_vendor: AtomicU16,
    id_product: AtomicU16,
}

impl UsbGadget {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // Add attributes commonly found in a USB device
        builder.add(SysStr::from("idVendor"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        builder.add(SysStr::from("idProduct"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            // Every gadget needs a 'config' directory
            fields
                .add_child(UsbConfig::new())
                .expect("Failed to add configs directory");
            UsbGadget {
                fields,
                id_vendor: AtomicU16::new(0),
                id_product: AtomicU16::new(0),
            }
        })
    }
}

inherit_sys_branch_node!(UsbGadget, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "idVendor" => {
                writer
                    .write_val(&self.id_vendor.load(Ordering::Relaxed))
                    .unwrap();
                Ok(size_of::<u16>())
            }
            "idProduct" => {
                writer
                    .write_val(&self.id_product.load(Ordering::Relaxed))
                    .unwrap();
                Ok(size_of::<u16>())
            }
            _ => Err(Error::AttributeError),
        }
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "idVendor" => {
                let val = reader.read_val::<u16>().unwrap();
                self.id_vendor.store(val, Ordering::Relaxed);
                Ok(size_of::<u16>())
            }
            "idProduct" => {
                let val = reader.read_val::<u16>().unwrap();
                self.id_product.store(val, Ordering::Relaxed);
                Ok(size_of::<u16>())
            }
            _ => Err(Error::AttributeError),
        }
    }
});

/// A specific USB configuration.
#[derive(Debug)]
struct UsbConfig {
    fields: NormalNodeFields<Self>,
    max_power: AtomicU16,
}

impl UsbConfig {
    fn new() -> Arc<Self> {
        let name = SysStr::from("config");
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("MaxPower"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());
            UsbConfig {
                fields,
                max_power: AtomicU16::new(0),
            }
        })
    }
}

inherit_sys_leaf_node!(UsbConfig, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        if name == "MaxPower" {
            writer
                .write_val(&self.max_power.load(Ordering::Relaxed))
                .unwrap();
            return Ok(size_of::<u16>());
        }

        Err(Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        if name == "MaxPower" {
            let new_power = reader.read_val::<u16>().unwrap();
            self.max_power.store(new_power, Ordering::Relaxed);
            return Ok(size_of::<u16>());
        }

        Err(Error::AttributeError)
    }
});

// --- Test Setup ---

static USB_GADGET_SUBSYSTEM: Once<Arc<UsbGadgetSystem>> = Once::new();

fn init_usb_gadget_subsystem() {
    if USB_GADGET_SUBSYSTEM.is_completed() {
        return;
    }

    time_init_for_ktest();
    super::init_for_ktest();

    let usb_gadget_system = UsbGadgetSystem::new();
    USB_GADGET_SUBSYSTEM.call_once(|| usb_gadget_system.clone());
    super::register_subsystem(usb_gadget_system);
}

#[ktest]
fn test_config_fs() {
    init_usb_gadget_subsystem();
    let config_fs = super::singleton();
    // path: /sys/kernel/config
    let root_inode = config_fs.root_inode();

    // --- Create Gadget and Configuration ---
    // path: /sys/kernel/config/usb_gadget
    let usb_gadget_system = root_inode
        .lookup("usb_gadget")
        .expect("lookup usb_gadget failed");
    // path: /sys/kernel/config/usb_gadget/g1
    let gadget_1 = usb_gadget_system
        .create("g1", InodeType::Dir, InodeMode::from_bits_truncate(0o755))
        .expect("creating gadget 'g1' fails");
    // path: /sys/kernel/config/usb_gadget/g1/config
    let config = gadget_1.lookup("config").expect("lookup config failed");

    // --- Read/Write Attributes ---
    let id_vendor = gadget_1.lookup("idVendor").expect("lookup idVendor failed");
    let max_power = config.lookup("MaxPower").expect("lookup MaxPower failed");

    let mut read_buffer: u16 = 0;

    // Test idVendor read/write.
    assert!(id_vendor
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 0);

    let write_buffer_vid: u16 = 0x1d6b;
    assert!(id_vendor
        .write_bytes_at(0, write_buffer_vid.as_bytes())
        .is_ok());
    assert!(id_vendor
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 0x1d6b);

    // Test MaxPower read/write.
    assert!(max_power
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 0);

    let write_buffer_power: u16 = 250;
    assert!(max_power
        .write_bytes_at(0, write_buffer_power.as_bytes())
        .is_ok());
    assert!(max_power
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 250);
}
