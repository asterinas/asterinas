// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{
    BranchNodeFields, Error, NormalNodeFields, Result, SysAttrSetBuilder, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node, inherit_sys_leaf_node,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

pub(super) fn init() {
    let node = {
        let tdx_guest_node = TdxGuestSysNodeRoot::new();
        let measurement = Measurement::new(SysStr::from("measurements"));
        tdx_guest_node.add_child(measurement).unwrap();

        // FIXME: Temporary folder node until we have a proper sysfs devices
        // implementation.
        let misc_node = FolderNode::new("misc");
        misc_node.add_child(tdx_guest_node).unwrap();
        let virtual_node = FolderNode::new("virtual");
        virtual_node.add_child(misc_node).unwrap();
        let devices_node = FolderNode::new("devices");
        devices_node.add_child(virtual_node).unwrap();

        devices_node
    };

    crate::fs::sysfs::systree_singleton()
        .root()
        .add_child(node.clone())
        .unwrap();
}

/// A systree node representing the `/sys/devices/virtual/misc/tdx_guest`
/// directory.
#[derive(Debug)]
pub struct TdxGuestSysNodeRoot {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl TdxGuestSysNodeRoot {
    fn new() -> Arc<Self> {
        let name = SysStr::from("tdx_guest");
        let attrs = SysAttrSetBuilder::new().build().unwrap();
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());

            TdxGuestSysNodeRoot { fields }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        self.fields.add_child(new_child)
    }
}

inherit_sys_branch_node!(TdxGuestSysNodeRoot, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[derive(Debug)]
struct Measurement {
    fields: NormalNodeFields<Self>,
}

#[derive(Debug)]
struct MeasurementAttr {
    name: &'static str,
    perms: SysPerms,
}

const MEASUREMENT_ATTRS: &[MeasurementAttr] = &[
    MeasurementAttr {
        name: "mrconfigid",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "mrowner",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "mrownerconfig",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "mrtd:sha384",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "rtmr0:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "rtmr1:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "rtmr2:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
    },
    MeasurementAttr {
        name: "rtmr3:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
    },
];

impl Measurement {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        for attr in MEASUREMENT_ATTRS {
            builder.add(SysStr::from(attr.name), attr.perms);
        }
        let attrs = builder.build().unwrap();

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());

            Measurement { fields }
        })
    }
}

inherit_sys_leaf_node!(Measurement, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
});

#[derive(Debug)]
struct FolderNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl FolderNode {
    fn new(name: &'static str) -> Arc<Self> {
        let name = SysStr::from(name);
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(
                name,
                SysAttrSetBuilder::new().build().unwrap(),
                weak_self.clone(),
            );

            FolderNode { fields }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        self.fields.add_child(new_child)
    }
}

inherit_sys_branch_node!(FolderNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
