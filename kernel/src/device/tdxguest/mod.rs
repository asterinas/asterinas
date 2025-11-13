// SPDX-License-Identifier: MPL-2.0

use core::{mem::size_of, time::Duration};

use align_ext::AlignExt;
use aster_util::{field_ptr, safe_ptr::SafePtr};
use device_id::DeviceId;
use ostd::{
    mm::{DmaCoherent, FrameAllocOptions, HasPaddr, HasSize, VmIo, PAGE_SIZE},
    sync::WaitQueue,
};
use tdx_guest::{
    tdcall::{get_report, TdCallError},
    tdvmcall::{get_quote, TdVmcallError},
    SHARED_MASK,
};

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::{IoctlCmd, StatusFlags},
    },
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

pub struct TdxGuest;

impl Device for TdxGuest {
    fn type_(&self) -> DeviceType {
        DeviceType::Misc
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

impl From<TdVmcallError> for Error {
    fn from(err: TdVmcallError) -> Self {
        match err {
            TdVmcallError::TdxRetry => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxRetry")
            }
            TdVmcallError::TdxOperandInvalid => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxOperandInvalid")
            }
            TdVmcallError::TdxGpaInuse => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxGpaInuse")
            }
            TdVmcallError::TdxAlignError => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxAlignError")
            }
            TdVmcallError::Other => Error::with_message(Errno::EAGAIN, "TdVmcallError::Other"),
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
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Read operation not supported")
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Write operation not supported")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TDXGETREPORT => handle_get_report(arg),
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }
}

pub fn tdx_get_quote(inblob: &[u8]) -> Result<Box<[u8]>> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    const GET_QUOTE_BUF_SIZE: usize = 8 * 1024;

    let report = tdx_get_report(inblob)?;
    let buf = alloc_dma_buf(GET_QUOTE_BUF_SIZE)?;
    let report_ptr: SafePtr<TdxQuoteHdr, _, _> = SafePtr::new(&buf, 0);

    field_ptr!(&report_ptr, TdxQuoteHdr, version).write(&1u64)?;
    field_ptr!(&report_ptr, TdxQuoteHdr, status).write(&0u64)?;
    field_ptr!(&report_ptr, TdxQuoteHdr, in_len).write(&(TDX_REPORT_LEN as u32))?;
    field_ptr!(&report_ptr, TdxQuoteHdr, out_len).write(&0u32)?;
    buf.write_bytes(size_of::<TdxQuoteHdr>(), &report)?;

    // FIXME: The `get_quote` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    get_quote((buf.paddr() as u64) | SHARED_MASK, buf.size() as u64)?;

    // Poll for the quote to be ready.
    let status_ptr = field_ptr!(&report_ptr, TdxQuoteHdr, status);
    let sleep_queue = WaitQueue::new();
    let sleep_duration = Duration::from_millis(100);
    loop {
        let status = status_ptr.read()?;
        if status != GET_QUOTE_IN_FLIGHT {
            break;
        }
        let _ = sleep_queue.wait_until_or_timeout(|| -> Option<()> { None }, &sleep_duration);
    }

    // Note: We cannot convert `DmaCoherent` to `USegment` here. When shared memory is converted back
    // to private memory in TDX, `TDG.MEM.PAGE.ACCEPT` will zero out all content.
    // TDX Module Specification - `TDG.MEM.PAGE.ACCEPT` Leaf:
    // "Accept a pending private page and initialize it to all-0 using the TD ephemeral private key."
    let out_len = field_ptr!(&report_ptr, TdxQuoteHdr, out_len).read()?;
    let mut outblob = vec![0u8; out_len as usize].into_boxed_slice();
    buf.read_bytes(size_of::<TdxQuoteHdr>(), outblob.as_mut())?;
    Ok(outblob)
}

#[repr(C)]
struct TdxQuoteHdr {
    // Quote version, filled by TD
    version: u64,
    // Status code of quote request, filled by VMM
    status: u64,
    // Length of TDREPORT, filled by TD
    in_len: u32,
    // Length of quote, filled by VMM
    out_len: u32,
}

fn handle_get_report(arg: usize) -> Result<i32> {
    let current_task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(current_task.as_thread_local().unwrap());
    let user_request: TdxReportRequest = user_space.read_val(arg)?;

    let report = tdx_get_report(&user_request.report_data)?;

    let tdx_report_vaddr = arg + TDX_REPORTDATA_LEN;
    user_space.write_bytes(tdx_report_vaddr, &mut VmReader::from(report.as_ref()))?;
    Ok(0)
}

fn tdx_get_report(inblob: &[u8]) -> Result<Box<[u8]>> {
    if inblob.len() != TDX_REPORTDATA_LEN {
        return_errno_with_message!(Errno::EINVAL, "Invalid inblob length");
    }

    let segment = FrameAllocOptions::new().alloc_segment(2)?;
    let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();
    dma_coherent.write_bytes(0, &inblob).unwrap();

    // FIXME: The `get_report` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    get_report(
        ((dma_coherent.paddr() + 1024) as u64) | SHARED_MASK,
        (dma_coherent.paddr() as u64) | SHARED_MASK,
    )?;

    // Note: We cannot convert `DmaCoherent` to `USegment` here. When shared memory is converted back
    // to private memory in TDX, `TDG.MEM.PAGE.ACCEPT` will zero out all content.
    // TDX Module Specification - `TDG.MEM.PAGE.ACCEPT` Leaf:
    // "Accept a pending private page and initialize it to all-0 using the TD ephemeral private key."
    let mut generated_report = Box::new([0u8; TDX_REPORT_LEN]);
    dma_coherent
        .read_bytes(1024, generated_report.as_mut())
        .unwrap();

    Ok(generated_report)
}

fn alloc_dma_buf(buf_len: usize) -> Result<DmaCoherent> {
    let aligned_buf_len = buf_len.align_up(PAGE_SIZE);
    let segment = FrameAllocOptions::new().alloc_segment(aligned_buf_len / PAGE_SIZE)?;

    let dma_buf = DmaCoherent::map(segment.into(), false).unwrap();
    Ok(dma_buf)
}
