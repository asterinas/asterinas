// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::ToString, sync::Arc};
use core::fmt::Debug;

use aster_systree::{
    inherit_sys_branch_node, inherit_sys_leaf_node, BranchNodeFields, Error, NormalNodeFields,
    Result, SysAttrSetBuilder, SysObj, SysPerms, SysStr,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    sync::RwLock,
};
use spin::Once;

use crate::device::tdxguest::tdx_get_quote;

const TSM_INBLOB_MAX: usize = 64;

#[derive(Debug)]
struct Tsm {
    fields: BranchNodeFields<ReportSystem, Self>,
}

impl Tsm {
    fn new() -> Arc<Self> {
        let name = SysStr::from("tsm");
        let builder = SysAttrSetBuilder::new();

        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            fields
                .add_child(ReportSystem::new())
                .expect("Failed to add report directory");

            Tsm { fields }
        })
    }
}

inherit_sys_branch_node!(Tsm, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[derive(Debug)]
struct ReportSystem {
    fields: BranchNodeFields<Report, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ReportSystem {
    fn new() -> Arc<Self> {
        let name = SysStr::from("report");
        let builder = SysAttrSetBuilder::new();

        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| ReportSystem {
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }

    fn add_child(&self, new_child: Arc<Report>) -> Result<()>;
}

inherit_sys_branch_node!(ReportSystem, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let report = Report::new(SysStr::from(name.to_string()));
        self.add_child(report.clone())?;
        Ok(report)
    }
});

#[derive(Debug)]
pub struct TsmProvider {
    provider: SysStr,
    inblob: [u8; TSM_INBLOB_MAX],
    inblob_len: usize,
    outblob: Option<Box<[u8]>>,
    generation: u32,
}

impl TsmProvider {
    pub fn new() -> Self {
        TsmProvider {
            provider: SysStr::from("tdx_guest"),
            inblob: [0u8; TSM_INBLOB_MAX],
            inblob_len: 0,
            outblob: None,
            generation: 1,
        }
    }
}

#[derive(Debug)]
struct Report {
    fields: NormalNodeFields<Self>,
    data: RwLock<TsmProvider>,
}

impl Report {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("inblob"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        builder.add(SysStr::from("outblob"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("provider"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("generation"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());

            Report {
                fields,
                data: RwLock::new(TsmProvider::new()),
            }
        })
    }
}

inherit_sys_leaf_node!(Report, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "provider" => {
                let mut printer = VmPrinter::new_skip(writer, offset);
                write!(printer, "{}\n", self.data.read().provider)?;
                return Ok(printer.bytes_written());
            }

            "outblob" => {
                let mut data = self.data.write();

                if data.inblob_len != TSM_INBLOB_MAX {
                    return Err(Error::AttributeError);
                }

                if offset == 0 {
                    data.outblob =
                        Some(tdx_get_quote(&data.inblob).map_err(|_| Error::AttributeError)?);
                }

                let outblob = data.outblob.as_ref().ok_or(Error::AttributeError)?;

                let slice = &outblob[offset..];
                let write_len = writer
                    .write_fallible(&mut slice.into())
                    .map_err(|_| Error::AttributeError)?;
                return Ok(write_len);
            }

            "generation" => {
                let mut printer = VmPrinter::new_skip(writer, offset);
                write!(printer, "{}\n", self.data.read().generation)?;
                return Ok(printer.bytes_written());
            }

            _ => {}
        }

        Err(Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        if name == "inblob" {
            let mut data = self.data.write();

            let mut writer = VmWriter::from(&mut data.inblob[..]);
            let read_len = reader
                .read_fallible(&mut writer)
                .map_err(|_| Error::AttributeError)?;

            data.inblob_len = read_len;
            data.generation = data.generation.wrapping_add(1);

            return Ok(read_len);
        }

        Err(Error::AttributeError)
    }
});

static TSM_SUBSYSTEM: Once<Arc<Tsm>> = Once::new();

pub fn init_tsm_subsystem() {
    if TSM_SUBSYSTEM.is_completed() {
        return;
    }

    let tsm = Tsm::new();
    TSM_SUBSYSTEM.call_once(|| tsm.clone());
    super::register_subsystem(tsm).unwrap();
}
