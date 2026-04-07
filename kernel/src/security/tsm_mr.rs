// SPDX-License-Identifier: MPL-2.0

//! TDX measurement registers (TSM-MR) sysfs interface.
//!
//! This module exposes Intel TDX measurement registers to userspace under
//! `/sys/devices/virtual/misc/tdx_guest/measurements/`. Each file corresponds
//! to one register:
//!
//! | Path | Register | Writable |
//! |------|----------|----------|
//! | `mrconfigid` | `MRCONFIGID` | No |
//! | `mrowner` | `MROWNER` | No |
//! | `mrownerconfig` | `MROWNERCONFIG` | No |
//! | `mrtd:sha384` | `MRTD` | No |
//! | `rtmr0:sha384` … `rtmr3:sha384` | `RTMR[0..3]` | Yes |
//!
//! For more information about the TSM-MR ABI, see the Linux kernel
//! [documentation](https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-devices-virtual-misc-tdx_guest).

use alloc::sync::Arc;

use aster_systree::{
    BranchNodeFields, Error, NormalNodeFields, Result, SysAttrSetBuilder, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node, inherit_sys_leaf_node,
};
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    sync::RwMutex,
};

use crate::device::misc::tdxguest::{self, MeasurementReg, Rtmr, SHA384_DIGEST_SIZE};

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
struct TdxGuestSysNodeRoot {
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
    /// Whether the cached TDX report is in sync with current hardware state.
    ///
    /// Set to `false` by `write_attr` after a successful
    /// [`tdxguest::extend_tdx_mr`] call, because the extend changes RTMR
    /// hardware state that is not yet reflected in the cached
    /// `TDREPORT_STRUCT`.  Set back to `true` by `read_attr_at` once it has
    /// re-generated the report via [`tdxguest::get_tdx_mr_refresh`].
    ///
    /// Invariant: when `in_sync` is `true`, the cached report correctly
    /// reflects the current value of every RTMR.  Static registers
    /// (`MRCONFIGID`, `MROWNER`, `MROWNERCONFIG`, `MRTD`) are fixed at TD
    /// creation time and are always in sync regardless of this flag.
    in_sync: RwMutex<bool>,
}

#[derive(Debug)]
struct MeasurementAttr {
    name: &'static str,
    perms: SysPerms,
    reg: MeasurementReg,
    /// Whether reading this attribute requires refreshing the cached TDX report
    /// first.
    ///
    /// Static registers (`MRCONFIGID`, `MROWNER`, `MROWNERCONFIG`, `MRTD`) are
    /// written by the TDX module at TD creation time and can never change at
    /// runtime, so `false` is correct for them — the cached `TDREPORT_STRUCT`
    /// is always authoritative.
    ///
    /// RTMRs can be extended at any time via [`tdxguest::extend_tdx_mr`], which
    /// changes hardware state without updating the cache.  For these registers
    /// `refresh_on_read` is `true`, and `read_attr_at` will conditionally
    /// re-generate the report (via [`tdxguest::get_tdx_mr_refresh`]) whenever
    /// [`Measurement::in_sync`] is `false`.
    refresh_on_read: bool,
}

impl MeasurementAttr {
    fn refresh_on_read(&self) -> bool {
        self.refresh_on_read
    }
}

const MEASUREMENT_ATTRS: &[MeasurementAttr] = &[
    MeasurementAttr {
        name: "mrconfigid",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
        reg: MeasurementReg::MrConfigId,
        refresh_on_read: false,
    },
    MeasurementAttr {
        name: "mrowner",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
        reg: MeasurementReg::MrOwner,
        refresh_on_read: false,
    },
    MeasurementAttr {
        name: "mrownerconfig",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
        reg: MeasurementReg::MrOwnerConfig,
        refresh_on_read: false,
    },
    MeasurementAttr {
        name: "mrtd:sha384",
        perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
        reg: MeasurementReg::MrTd,
        refresh_on_read: false,
    },
    MeasurementAttr {
        name: "rtmr0:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
        reg: MeasurementReg::Rtmr(Rtmr::Rtmr0),
        refresh_on_read: true,
    },
    MeasurementAttr {
        name: "rtmr1:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
        reg: MeasurementReg::Rtmr(Rtmr::Rtmr1),
        refresh_on_read: true,
    },
    MeasurementAttr {
        name: "rtmr2:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
        reg: MeasurementReg::Rtmr(Rtmr::Rtmr2),
        refresh_on_read: true,
    },
    MeasurementAttr {
        name: "rtmr3:sha384",
        perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
        reg: MeasurementReg::Rtmr(Rtmr::Rtmr3),
        refresh_on_read: true,
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

            Measurement {
                fields,
                in_sync: RwMutex::new(true),
            }
        })
    }
}

inherit_sys_leaf_node!(Measurement, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let attr = MEASUREMENT_ATTRS
            .iter()
            .find(|attr| attr.name == name)
            .ok_or(Error::NotFound)?;

        let mut in_sync = self.in_sync.upread();

        let mr = (if attr.refresh_on_read() && !*in_sync {
            let mut in_sync_write = in_sync.upgrade();
            let mr = tdxguest::get_tdx_mr_refresh(attr.reg);
            *in_sync_write = true;
            in_sync = in_sync_write.downgrade();
            mr
        } else {
            tdxguest::get_tdx_mr(attr.reg)
        })
        .map_err(|_| Error::AttributeError)?;

        if offset >= mr.len() {
            return Ok(0);
        }
        let mut reader = VmReader::from(&mr[offset..]);
        writer
            .write_fallible(&mut reader)
            .map_err(|_| Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        let attr = MEASUREMENT_ATTRS
            .iter()
            .find(|attr| attr.name == name)
            .ok_or(Error::NotFound)?;

        let MeasurementReg::Rtmr(rtmr) = attr.reg else {
            return Err(Error::PermissionDenied);
        };

        let data = {
            let mut buf = [0u8; SHA384_DIGEST_SIZE];
            let mut writer = VmWriter::from(&mut buf[..]);
            let bytes_read = reader
                .read_fallible(&mut writer)
                .map_err(|_| Error::AttributeError)?;
            if bytes_read != buf.len() {
                return Err(Error::InvalidOperation);
            }

            buf
        };

        let mut in_sync = self.in_sync.write();

        tdxguest::extend_tdx_mr(rtmr, &data).map_err(|_| Error::AttributeError)?;

        if attr.refresh_on_read() {
            *in_sync = false;
        }

        Ok(data.len())
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
