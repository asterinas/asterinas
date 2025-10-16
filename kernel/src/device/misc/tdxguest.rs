// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, SysAttrSetBuilder, SysBranchNode, SysPerms, SysStr,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{DmaCoherent, FrameAllocOptions, HasPaddr, VmIo};
use tdx_guest::tdcall::{get_report, TdCallError};

use super::MISC_ID_ALLOCATOR;
use crate::{
    events::IoEvents,
    fs::{device::DeviceFile, inode_handle::FileIo, utils::IoctlCmd},
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

const TDX_REPORTDATA_LEN: usize = 64;
const TDX_REPORT_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct TdxReportRequest {
    report_data: [u8; TDX_REPORTDATA_LEN],
    tdx_report: [u8; TDX_REPORT_LEN],
}

const TDX_GUEST_MINOR: u32 = 0x7b;

/// The `/dev/tdx_guest` device.
#[derive(Debug)]
pub struct TdxGuest {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for TdxGuest {
    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn device_id(&self) -> Option<DeviceId> {
        Some(self.id)
    }
}

inherit_sys_branch_node!(TdxGuest, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        // Check if attribute exists
        if !self.fields.attr_set().contains(name) {
            return Err(aster_systree::Error::NotFound);
        }

        let attr = self.fields.attr_set().get(name).unwrap();
        // Check if attribute is readable
        if !attr.perms().can_read() {
            return Err(aster_systree::Error::PermissionDenied);
        }

        let mut printer = VmPrinter::new_skip(writer, offset);
        if name == "dev" {
            writeln!(printer, "{}:{}", self.id.major(), self.id.minor())
                .map_err(|_| aster_systree::Error::AttributeError)?
        };

        Ok(printer.bytes_written())
    }
});

#[inherit_methods(from = "self.fields")]
impl TdxGuest {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> aster_systree::Result<()>;
    pub fn remove_child(&self, child_name: &str) -> aster_systree::Result<Arc<dyn SysBranchNode>>;
}

impl TdxGuest {
    pub(super) fn new() -> Arc<Self> {
        let id = MISC_ID_ALLOCATOR
            .get()
            .unwrap()
            .allocate(TDX_GUEST_MINOR)
            .unwrap();
        let name = SysStr::from("tdx_guest");

        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| TdxGuest {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl Drop for TdxGuest {
    fn drop(&mut self) {
        MISC_ID_ALLOCATOR.get().unwrap().release(self.id.minor());
    }
}

impl From<TdCallError> for Error {
    fn from(err: TdCallError) -> Self {
        match err {
            TdCallError::TdxNoValidVeInfo => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxNoValidVeInfo")
            }
            TdCallError::TdxOperandInvalid => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxOperandInvalid")
            }
            TdCallError::TdxPageAlreadyAccepted => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxPageAlreadyAccepted")
            }
            TdCallError::TdxPageSizeMismatch => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxPageSizeMismatch")
            }
            TdCallError::TdxOperandBusy => {
                Error::with_message(Errno::EBUSY, "TdCallError::TdxOperandBusy")
            }
            TdCallError::Other => Error::with_message(Errno::EAGAIN, "TdCallError::Other"),
            _ => todo!(),
        }
    }
}

impl Pollable for TdxGuest {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for TdxGuest {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Read operation not supported")
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Write operation not supported")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TDXGETREPORT => handle_get_report(arg),
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }
}

impl DeviceFile for TdxGuest {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(self.weak_self().upgrade().unwrap()))
    }
}

fn handle_get_report(arg: usize) -> Result<i32> {
    const SHARED_BIT: u8 = 51;
    const SHARED_MASK: u64 = 1u64 << SHARED_BIT;
    let current_task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(current_task.as_thread_local().unwrap());
    let user_request: TdxReportRequest = user_space.read_val(arg)?;

    let segment = FrameAllocOptions::new().alloc_segment(2).unwrap();
    let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();
    dma_coherent
        .write_bytes(0, &user_request.report_data)
        .unwrap();
    // 1024-byte alignment.
    dma_coherent
        .write_bytes(1024, &user_request.tdx_report)
        .unwrap();

    if let Err(err) = get_report(
        ((dma_coherent.paddr() + 1024) as u64) | SHARED_MASK,
        (dma_coherent.paddr() as u64) | SHARED_MASK,
    ) {
        println!("[kernel]: get TDX report error: {:?}", err);
        return Err(err.into());
    }

    let tdx_report_vaddr = arg + TDX_REPORTDATA_LEN;
    let mut generated_report = vec![0u8; TDX_REPORT_LEN];
    dma_coherent
        .read_bytes(1024, &mut generated_report)
        .unwrap();
    let report_slice: &[u8] = &generated_report;
    user_space.write_bytes(tdx_report_vaddr, &mut VmReader::from(report_slice))?;
    Ok(0)
}
