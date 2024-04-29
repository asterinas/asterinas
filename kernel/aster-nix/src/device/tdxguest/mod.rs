// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use aster_frame::vm::{vaddr_to_paddr, DmaCoherent, HasPaddr, VmAllocOptions, VmIo, PAGE_SIZE};
use tdx_guest::{
    tdcall::{extend_rtmr, get_report, TdCallError},
    tdvmcall::{get_quote, TdVmcallError},
    SHARED_MASK,
};

use super::*;
use crate::{
    error::Error,
    events::IoEvents,
    fs::{inode_handle::FileIo, utils::IoctlCmd},
    process::signal::Poller,
    util::{read_bytes_from_user, read_val_from_user, write_bytes_to_user},
};

const TDX_REPORTDATA_LEN: usize = 64;
const TDX_REPORT_LEN: usize = 1024;
const TDX_EXTEND_RTMR_DATA_LEN: usize = 48;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct TdxReportRequest {
    reportdata: [u8; TDX_REPORTDATA_LEN],
    tdreport: [u8; TDX_REPORT_LEN],
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct TdxQuoteRequest {
    buf: usize,
    len: usize,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct TdxExtendRtmrReq {
    data: [u8; TDX_EXTEND_RTMR_DATA_LEN],
    index: u8,
}

#[repr(align(64))]
#[repr(C)]
struct ReportDataWapper {
    report_data: [u8; TDX_REPORTDATA_LEN],
}

#[repr(align(1024))]
#[repr(C)]
struct TdxReportWapper {
    tdx_report: [u8; TDX_REPORT_LEN],
}

#[repr(align(64))]
#[repr(C)]
struct ExtendRtmrWapper {
    data: [u8; TDX_EXTEND_RTMR_DATA_LEN],
    index: u8,
}

struct QuoteEntry {
    // Kernel buffer to share data with VMM (size is page aligned)
    buf: DmaCoherent,
    // Size of the allocated memory
    buf_len: usize,
}

#[repr(C)]
struct tdx_quote_hdr {
    // Quote version, filled by TD
    version: u64,
    // Status code of Quote request, filled by VMM
    status: u64,
    // Length of TDREPORT, filled by TD
    in_len: u32,
    // Length of Quote, filled by VMM
    out_len: u32,
    // Actual Quote data or TDREPORT on input
    data: Vec<u64>,
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

impl FileIo for TdxGuest {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Read operation not supported")
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Write operation not supported")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TDXGETREPORT => handle_get_report(arg),
            IoctlCmd::TDXGETQUOTE => handle_get_quote(arg),
            IoctlCmd::TDXEXTENDRTMR => handle_extend_rtmr(arg),
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

fn handle_get_report(arg: usize) -> Result<i32> {
    let tdx_report: TdxReportRequest = read_val_from_user(arg)?;
    let wrapped_report = TdxReportWapper {
        tdx_report: tdx_report.tdreport,
    };
    let wrapped_data = ReportDataWapper {
        report_data: tdx_report.reportdata,
    };
    if let Err(err) = get_report(
        vaddr_to_paddr(wrapped_report.tdx_report.as_ptr() as usize).unwrap() as u64,
        vaddr_to_paddr(wrapped_data.report_data.as_ptr() as usize).unwrap() as u64,
    ) {
        println!("[kernel]: get TDX report error");
        return Err(err.into());
    }
    let tdx_report_vaddr = arg + TDX_REPORTDATA_LEN;
    write_bytes_to_user(tdx_report_vaddr, &wrapped_report.tdx_report)?;
    Ok(0)
}

fn handle_get_quote(arg: usize) -> Result<i32> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    let tdx_quote: TdxQuoteRequest = read_val_from_user(arg)?;
    if tdx_quote.len == 0 {
        return Err(Error::with_message(Errno::EBUSY, "Invalid parameter"));
    }
    let entry = alloc_quote_entry(tdx_quote.len);

    // Copy data (with TDREPORT) from user buffer to kernel Quote buffer
    let mut quote_buffer = vec![0u8; entry.buf_len];
    read_bytes_from_user(tdx_quote.buf, &mut quote_buffer)?;
    entry.buf.write_bytes(0, &quote_buffer)?;

    if let Err(err) = get_quote(
        (entry.buf.paddr() as u64) | SHARED_MASK,
        entry.buf_len as u64,
    ) {
        println!("[kernel] get quote error: {:?}", err);
        return Err(err.into());
    }

    // Poll for the quote to be ready.
    loop {
        entry.buf.read_bytes(0, &mut quote_buffer)?;
        let quote_hdr: tdx_quote_hdr = parse_quote_header(&quote_buffer);
        if quote_hdr.status != GET_QUOTE_IN_FLIGHT {
            break;
        }
    }
    entry.buf.read_bytes(0, &mut quote_buffer)?;
    write_bytes_to_user(tdx_quote.buf, &mut quote_buffer)?;
    Ok(0)
}

fn handle_extend_rtmr(arg: usize) -> Result<i32> {
    let extend_rtmr_req: TdxExtendRtmrReq = read_val_from_user(arg)?;
    if extend_rtmr_req.index < 2 {
        return Err(Error::with_message(Errno::EINVAL, "Invalid parameter"));
    }
    let wrapped_extend_rtmr = ExtendRtmrWapper {
        data: extend_rtmr_req.data,
        index: extend_rtmr_req.index,
    };
    if let Err(err) = extend_rtmr(
        vaddr_to_paddr(wrapped_extend_rtmr.data.as_ptr() as usize).unwrap() as u64,
        wrapped_extend_rtmr.index as u64,
    ) {
        println!("[kernel]: TDX extend rtmr error");
        return Err(err.into());
    }
    Ok(0)
}

fn alloc_quote_entry(buf_len: usize) -> QuoteEntry {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    let aligned_buf_len = buf_len & (!PAGE_MASK);
    let dma_buf: DmaCoherent = DmaCoherent::map(
        VmAllocOptions::new(aligned_buf_len / (PAGE_SIZE))
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap(),
        true,
    )
    .unwrap();
    let entry = QuoteEntry {
        buf: dma_buf,
        buf_len: aligned_buf_len as usize,
    };
    entry
}

fn parse_quote_header(buffer: &[u8]) -> tdx_quote_hdr {
    let version = u64::from_be_bytes(buffer[0..8].try_into().unwrap());
    let status = u64::from_be_bytes(buffer[8..16].try_into().unwrap());
    let in_len = u32::from_be_bytes(buffer[16..20].try_into().unwrap());
    let out_len = u32::from_be_bytes(buffer[20..24].try_into().unwrap());

    tdx_quote_hdr {
        version,
        status,
        in_len,
        out_len,
        data: vec![0],
    }
}
