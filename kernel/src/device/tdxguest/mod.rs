// SPDX-License-Identifier: MPL-2.0

use core::mem::{offset_of, size_of};

use align_ext::AlignExt;
use ostd::mm::{DmaCoherent, FrameAllocOptions, HasPaddr, VmIo, PAGE_SIZE};
use tdx_guest::{
    tdcall::{get_report, TdCallError},
    tdvmcall::{get_quote, TdVmcallError},
    SHARED_MASK,
};

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
        utils::IoctlCmd,
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

struct QuoteEntry {
    // Kernel buffer to share data with VMM (size is page aligned)
    buf: DmaCoherent,
    // Size of the allocated memory
    buf_len: usize,
}

#[repr(C)]
struct TdxQuoteHdr {
    // Quote version, filled by TD
    version: u64,
    // Status code of Quote request, filled by VMM
    status: u64,
    // Length of TDREPORT, filled by TD
    in_len: u32,
    // Length of Quote, filled by VMM
    out_len: u32,
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

fn tdx_get_report(inblob: &[u8]) -> Result<Box<[u8]>> {
    let segment = FrameAllocOptions::new().alloc_segment(2).unwrap();
    let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();
    dma_coherent.write_bytes(0, &inblob).unwrap();

    if let Err(err) = get_report(
        ((dma_coherent.paddr() + 1024) as u64) | SHARED_MASK,
        (dma_coherent.paddr() as u64) | SHARED_MASK,
    ) {
        println!("[kernel]: get TDX report error: {:?}", err);
        return Err(err.into());
    }

    let mut generated_report = Box::new([0u8; TDX_REPORT_LEN]);
    dma_coherent
        .read_bytes(1024, generated_report.as_mut())
        .unwrap();

    Ok(generated_report)
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

pub fn tdx_get_quote(inblob: &[u8]) -> Result<Box<[u8]>> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    const GET_QUOTE_BUF_SIZE: usize = 8 * 1024;

    let report = tdx_get_report(inblob)?;
    let entry = alloc_quote_entry(GET_QUOTE_BUF_SIZE);

    entry
        .buf
        .write_bytes(offset_of!(TdxQuoteHdr, version), &1u64.to_le_bytes())?;
    entry
        .buf
        .write_bytes(offset_of!(TdxQuoteHdr, status), &0u64.to_le_bytes())?;
    entry.buf.write_bytes(
        offset_of!(TdxQuoteHdr, in_len),
        &(TDX_REPORT_LEN as u32).to_le_bytes(),
    )?;
    entry
        .buf
        .write_bytes(offset_of!(TdxQuoteHdr, out_len), &0u32.to_le_bytes())?;
    entry.buf.write_bytes(size_of::<TdxQuoteHdr>(), &report)?;

    if let Err(err) = get_quote(
        (entry.buf.paddr() as u64) | SHARED_MASK,
        entry.buf_len as u64,
    ) {
        error!("[kernel] get quote error: {:?}", err);
        return Err(err.into());
    }

    let mut quote_buffer = [0u8; size_of::<TdxQuoteHdr>()];
    let mut quote_hdr: TdxQuoteHdr;

    // Poll for the quote to be ready.
    loop {
        entry.buf.read_bytes(0, &mut quote_buffer)?;
        quote_hdr = parse_quote_header(&quote_buffer);
        if quote_hdr.status != GET_QUOTE_IN_FLIGHT {
            break;
        }
    }

    let mut outblob = vec![0u8; quote_hdr.out_len as usize].into_boxed_slice();
    entry
        .buf
        .read_bytes(size_of::<TdxQuoteHdr>(), outblob.as_mut())?;
    Ok(outblob)
}

fn alloc_quote_entry(buf_len: usize) -> QuoteEntry {
    let aligned_buf_len = buf_len.align_up(PAGE_SIZE);

    let segment = FrameAllocOptions::new()
        .alloc_segment(aligned_buf_len / PAGE_SIZE)
        .unwrap();
    let dma_buf = DmaCoherent::map(segment.into(), false).unwrap();

    let entry = QuoteEntry {
        buf: dma_buf,
        buf_len: aligned_buf_len,
    };
    entry
}

fn parse_quote_header(buffer: &[u8]) -> TdxQuoteHdr {
    let version = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
    let status = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
    let in_len = u32::from_le_bytes(buffer[16..20].try_into().unwrap());
    let out_len = u32::from_le_bytes(buffer[20..24].try_into().unwrap());

    TdxQuoteHdr {
        version,
        status,
        in_len,
        out_len,
    }
}
