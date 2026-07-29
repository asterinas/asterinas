// SPDX-License-Identifier: MPL-2.0

//! Trusted Security Manager (TSM).
//!
//! This module provides a cross-vendor ConfigFS interface
//! for generating confidential-computing attestation reports.
//! Userspace writes a request blob under `/sys/kernel/config/tsm/report/$name/inblob`
//! and reads back the signed report from `outblob`,
//! with optional knobs like `provider` and `generation`.
//!
//! For more information about the interface,
//! checkout Linux's [documentation](https://www.kernel.org/doc/Documentation/ABI/testing/configfs-tsm).

use alloc::{boxed::Box, string::ToString, sync::Arc};
use core::fmt::Debug;

use aster_systree::{
    BranchNodeFields, Error, NormalNodeFields, Result, SysAttrSetBuilder, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node, inherit_sys_leaf_node,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    sync::RwMutex,
};

use crate::{device::misc::tdxguest::tdx_get_quote, fs::configfs};

#[derive(Debug)]
struct Tsm {
    fields: BranchNodeFields<ReportSet, Self>,
}

impl Tsm {
    fn new() -> Arc<Self> {
        let name = SysStr::from("tsm");
        let attrs = SysAttrSetBuilder::new().build().unwrap();
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            fields.add_child(ReportSet::new()).unwrap();
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
struct ReportSet {
    fields: BranchNodeFields<ReportNode, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ReportSet {
    fn new() -> Arc<Self> {
        let name = SysStr::from("report");
        let attrs = SysAttrSetBuilder::new().build().unwrap();
        Arc::new_cyclic(|weak_self| ReportSet {
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }

    fn add_child(&self, new_child: Arc<ReportNode>) -> Result<()>;
}

inherit_sys_branch_node!(ReportSet, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let report = ReportNode::new(SysStr::from(name.to_string()));
        self.add_child(report.clone())?;
        Ok(report)
    }
});

const TSM_INBLOB_MAX: usize = 64;

/// The state machine of a report directory.
///
/// The states and their transitions are summarized in the table below.
///
/// ```text
/// ┌────────────────┐
/// │InblobNeeded    │
/// │(initial)       │
/// └────────┬───────┘
///          │
///          │ write inblob
/// ┌────────▼───────┐
/// │InblobProvided  ◄─────────┐
/// └────────┬───────┘         │
///          │                 │
///          │ read outblob    │ write inblob
/// ┌────────▼───────┐         │
/// │OutblobGenerated├─────────┘
/// └────────────────┘
/// ```
#[derive(Debug)]
enum TsmProviderState {
    /// Inblob is needed.
    ///
    /// This is the initial state.
    ///
    /// The behaviors of the files under a report directory
    /// are summarized in the table below:
    ///
    /// | Files        | Write behaviors| Read behaviors |
    /// |--------------|----------------|-----------------|
    /// | `inblob`     | Transist to `InblobProvided`  | Error due to WO |
    /// | `outblob`    | Error due to RO| Error (-EINVAL) |
    /// | `generation` | Error due to RO| Returns 0       |
    ///
    /// The initial value of `generation` is 0.
    InblobNeeded,
    /// Inblob is provided by the userspace, but outblob is not generated.
    ///
    /// The behaviors of the files under a report directory
    /// are summarized in the table below:
    ///
    /// | Files        | Write behaviors| Read behaviors |
    /// |--------------|----------------|-----------------|
    /// | `inblob`     | Update inblob  | Error due to WO |
    /// | `outblob`    | Error due to RO| Transist to `OutblobGenerated` |
    /// | `generation` | Error due to RO| Returns `generation` |
    ///
    /// Whenever @inblob or any option is written, the value of `generation`
    /// increases by 1.
    InblobProvided {
        generation: u32,
        inblob: [u8; TSM_INBLOB_MAX],
    },
    /// Outblob is generated and up-to-date.
    ///
    /// The behaviors of the files under a report directory
    /// are summarized in the table below:
    ///
    /// | Files        | Write behaviors| Read behaviors |
    /// |--------------|----------------|-----------------|
    /// | `inblob`     | Transist to `InblobProvided` | Error due to WO |
    /// | `outblob`    | Error due to RO| Returns `quote_buf` |
    /// | `generation` | Error due to RO| Returns `generation`|
    OutblobGenerated {
        generation: u32,
        quote_buf: Box<[u8]>,
    },
}

#[derive(Debug)]
struct TsmProvider {
    provider: SysStr,
    state: TsmProviderState,
}

impl TsmProvider {
    pub fn new() -> Self {
        TsmProvider {
            provider: SysStr::from("tdx_guest"),
            state: TsmProviderState::InblobNeeded,
        }
    }
}

#[derive(Debug)]
struct ReportNode {
    fields: NormalNodeFields<Self>,
    data: RwMutex<TsmProvider>,
}

impl ReportNode {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("inblob"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        builder.add(SysStr::from("outblob"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("provider"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("generation"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        let attrs = builder.build().unwrap();

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());
            ReportNode {
                fields,
                data: RwMutex::new(TsmProvider::new()),
            }
        })
    }

    fn read_inblob(&self, reader: &mut VmReader, inblob: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(&mut inblob[..]);
        reader
            .read_fallible(&mut writer)
            .map_err(|_| Error::AttributeError)
    }

    fn write_outblob(&self, writer: &mut VmWriter, outblob: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(outblob);
        writer
            .write_fallible(&mut reader)
            .map_err(|_| Error::AttributeError)
    }
}

inherit_sys_leaf_node!(ReportNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "provider" => {
                let data = self.data.read();
                let mut printer = VmPrinter::new_skip(writer, offset);
                writeln!(printer, "{}", data.provider)?;
                Ok(printer.bytes_written())
            }

            "outblob" => {
                let mut data = self.data.write();
                match data.state {
                    TsmProviderState::InblobNeeded => Err(Error::InvalidOperation),

                    TsmProviderState::InblobProvided {
                        ref inblob,
                        generation,
                    } => match tdx_get_quote(inblob) {
                        Ok(quote) => {
                            let res = self.write_outblob(writer, &quote[offset..]);

                            // Transition to `OutblobGenerated`
                            data.state = TsmProviderState::OutblobGenerated {
                                generation,
                                quote_buf: quote,
                            };
                            res
                        }
                        Err(_) => Err(Error::InvalidOperation),
                    },

                    TsmProviderState::OutblobGenerated { ref quote_buf, .. } => {
                        self.write_outblob(writer, &quote_buf[offset..])
                    }
                }
            }

            "generation" => {
                let data = self.data.read();
                let mut printer = VmPrinter::new_skip(writer, offset);
                let generation = match data.state {
                    TsmProviderState::InblobNeeded => 0,
                    TsmProviderState::InblobProvided { generation, .. } => generation,
                    TsmProviderState::OutblobGenerated { generation, .. } => generation,
                };
                writeln!(printer, "{}", generation)?;
                Ok(printer.bytes_written())
            }

            _ => Err(Error::AttributeError),
        }
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "inblob" => {
                let mut data = self.data.write();
                match data.state {
                    TsmProviderState::InblobNeeded => {
                        let mut inblob = [0u8; TSM_INBLOB_MAX];
                        let read_len = self.read_inblob(reader, &mut inblob)?;

                        // Transition to `InblobProvided`
                        data.state = TsmProviderState::InblobProvided {
                            generation: 1,
                            inblob,
                        };
                        Ok(read_len)
                    }

                    TsmProviderState::InblobProvided {
                        ref mut inblob,
                        ref mut generation,
                    } => {
                        let read_len = self.read_inblob(reader, inblob)?;
                        *generation = generation.wrapping_add(1);
                        Ok(read_len)
                    }

                    TsmProviderState::OutblobGenerated {
                        ref mut generation, ..
                    } => {
                        let mut inblob = [0u8; TSM_INBLOB_MAX];
                        let read_len = self.read_inblob(reader, &mut inblob)?;
                        *generation = generation.wrapping_add(1);

                        // Transition to `InblobProvided`
                        data.state = TsmProviderState::InblobProvided {
                            generation: *generation,
                            inblob,
                        };
                        Ok(read_len)
                    }
                }
            }

            _ => Err(Error::AttributeError),
        }
    }
});

pub(super) fn init() {
    configfs::register_subsystem(Tsm::new()).unwrap();
}
