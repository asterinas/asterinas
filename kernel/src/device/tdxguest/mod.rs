// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{DmaCoherent, FrameAllocOptions, HasPaddr, VmIo};
use tdx_guest::tdcall::{get_report, TdCallError};

use super::*;
use crate::{
    error::Error,
    events::IoEvents,
    fs::{inode_handle::FileIo, utils::IoctlCmd},
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

pub struct TdxGuest;

impl Device for TdxGuest {
    fn type_(&self) -> DeviceType {
        DeviceType::MiscDevice
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(0xa, 0x7b)
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

fn handle_get_report(arg: usize) -> Result<i32> {
    const SHARED_BIT: u8 = 51;
    const SHARED_MASK: u64 = 1u64 << SHARED_BIT;
    let current_task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(&current_task);
    let user_request: TdxReportRequest = user_space.read_val(arg)?;

    let vm_segment = FrameAllocOptions::new(2)
        .is_contiguous(true)
        .alloc_contiguous()
        .unwrap();
    let dma_coherent = DmaCoherent::map(vm_segment, false).unwrap();
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
