// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{
    mem::{offset_of, size_of},
    time::Duration,
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use device_id::{DeviceId, MinorId};
use ostd::{
    const_assert,
    mm::{
        FrameAllocOptions, HasPaddr, HasSize, PAGE_SIZE, USegment, VmIo, dma::DmaCoherent,
        io_util::HasVmReaderWriter,
    },
    sync::WaitQueue,
};
use tdx_guest::{
    SHARED_MASK,
    tdcall::{TdCallError, get_report},
    tdvmcall::{TdVmcallError, get_quote},
};

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

const TDX_GUEST_MINOR: u32 = 0x7b;

/// The `/dev/tdx_guest` device.
#[derive(Debug)]
pub struct TdxGuest {
    id: DeviceId,
}

impl TdxGuest {
    pub fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        let minor = MinorId::new(TDX_GUEST_MINOR);

        let id = DeviceId::new(major, minor);
        Arc::new(Self { id })
    }
}

impl Device for TdxGuest {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some("tdx_guest".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(TdxGuestFile))
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

struct TdxGuestFile;

impl Pollable for TdxGuestFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for TdxGuestFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the file is not valid for reading")
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the file not valid for writing")
    }
}

impl FileIo for TdxGuestFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "seek is not supported")
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetTdxReport => {
                cmd.with_data_ptr(|data_ptr| {
                    let inblob = {
                        let inblob_ptr = field_ptr!(&data_ptr, TdxReportRequest, report_data);
                        inblob_ptr.read()?
                    };

                    let outblob = tdx_get_report(inblob.as_bytes())?;
                    let outblob_ptr = field_ptr!(&data_ptr, TdxReportRequest, tdx_report);
                    outblob_ptr.copy_from(&outblob)?;

                    Ok(0)
                })
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown"),
        })
    }
}

pub fn tdx_get_quote(inblob: &[u8]) -> Result<Box<[u8]>> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    const GET_QUOTE_BUF_SIZE: usize = 8 * 1024;

    let report = tdx_get_report(inblob)?;

    let buf = alloc_dma_buf(GET_QUOTE_BUF_SIZE)?;
    let report_ptr: SafePtr<TdxQuoteHdr, _, _> = SafePtr::new(&buf, 0);
    let payload_ptr: SafePtr<TdReport, _, _> = SafePtr::new(&buf, size_of::<TdxQuoteHdr>());

    field_ptr!(&report_ptr, TdxQuoteHdr, version).write(&1u64)?;
    field_ptr!(&report_ptr, TdxQuoteHdr, status).write(&0u64)?;
    field_ptr!(&report_ptr, TdxQuoteHdr, in_len).write(&(size_of::<TdReport>() as u32))?;
    field_ptr!(&report_ptr, TdxQuoteHdr, out_len).write(&0u32)?;
    payload_ptr.copy_from(&report)?;

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
    payload_ptr.cast().read_slice(outblob.as_mut())?;
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

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct TdxReportRequest {
    report_data: ReportData,
    tdx_report: TdReport,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct ReportData {
    data: [u8; 64],
}

/// TDX Report structure (`TDREPORT_STRUCT`) as defined in the Intel TDX Module Specification.
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct TdReport {
    report_mac: ReportMac,
    _reserved: [u8; 256],
    td_info: TdInfo,
}
const_assert!(size_of::<TdReport>() == 1024);

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct ReportMac {
    _reserved1: [u8; 128],
    report_data: ReportData,
    _reserved2: [u8; 64],
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct TdInfo {
    attributes: [u8; 8],
    xfam: [u8; 8],
    mrtd: [u8; 48],
    mrconfigid: [u8; 48],
    mrowner: [u8; 48],
    mrownerconfig: [u8; 48],
    rtmr1: [u8; 48],
    rtmr2: [u8; 48],
    rtmr3: [u8; 48],
    rtmr4: [u8; 48],
    servtd_hash: [u8; 48],
    extension: [u8; 64],
}

impl TdReport {
    const fn report_data_offset() -> usize {
        offset_of!(TdReport, report_mac) + offset_of!(ReportMac, report_data)
    }
}

mod ioctl_defs {
    use super::TdxReportRequest;
    use crate::util::ioctl::{InOutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/tdx-guest.h#L40>

    pub(super) type GetTdxReport =
        ioc!(TDX_CMD_GET_REPORT0, b'T', 0x01, InOutData<TdxReportRequest>);
}

/// Gets the TDX report given the specified data in `inblob`.
fn tdx_get_report(inblob: &[u8]) -> Result<SafePtr<TdReport, USegment>> {
    if inblob.len() != size_of::<ReportData>() {
        return_errno_with_message!(Errno::EINVAL, "Invalid inblob length");
    }

    let report: USegment = {
        const REPORT_SIZE_IN_PAGES: usize = size_of::<TdReport>().div_ceil(PAGE_SIZE);
        FrameAllocOptions::new()
            .alloc_segment(REPORT_SIZE_IN_PAGES)?
            .into()
    };

    // Use `inblob` as the data associated with the report.
    let report_data_paddr = {
        // From TDX Module Specification, the report structure returned by TDX Module
        // places the report data at offset 128, so using the same offset keeps the
        // memory layout consistent with the TDX Modules's output format. And we can
        // directly call `get_report` on the existing report structure without needing
        // to rewrite the report data.
        report
            .write_bytes(TdReport::report_data_offset(), inblob)
            .unwrap();
        report.paddr() + TdReport::report_data_offset()
    };

    // FIXME: The `get_report` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    get_report(report.paddr() as u64, report_data_paddr as u64)?;
    Ok(SafePtr::new(report, 0))
}

fn alloc_dma_buf(buf_len: usize) -> Result<DmaCoherent> {
    let dma_buf = DmaCoherent::alloc(buf_len.div_ceil(PAGE_SIZE), false)?;
    Ok(dma_buf)
}
