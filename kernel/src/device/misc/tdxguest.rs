// SPDX-License-Identifier: MPL-2.0

//! Intel TDX guest device and measurement register API.
//!
//! This module provides two things:
//!
//! 1. **`/dev/tdx_guest` character device** — exposes the `TDX_CMD_GET_REPORT0`
//!    ioctl, which lets userspace request a `TDREPORT_STRUCT` from the TDX module
//!    (see [`TdxGuest`]).
//!
//! 2. **Measurement register API** — a set of public functions used by other
//!    kernel subsystems (e.g. the TSM-MR sysfs layer) to interact with TDX
//!    measurement registers.
//!
//! # Cached-report model
//!
//! A **TDX report** (`TDREPORT_STRUCT`) is a hardware-signed snapshot of the
//! TD's current measurement state, produced by the `TDG.MR.REPORT` TDCALL.
//! Throughout this module the word **"refresh"** always means issuing that
//! TDCALL to regenerate the snapshot; it never refers to individual register
//! reads.
//!
//! Because `TDG.MR.REPORT` is relatively expensive, the module keeps a single
//! **global cached report** in `TDX_REPORT` and satisfies measurement-register
//! reads from that cache.  The cache becomes **stale** after any
//! [`extend_tdx_mr`] call, because extending an RTMR changes hardware state
//! that is not reflected in the snapshot until the next refresh.
//!
//! # Measurement register API quick reference
//!
//! | Function | TDCALL? | Use when… |
//! |---|---|---|
//! | [`get_tdx_mr`] | No | Reading a register whose value is not expected to have changed since the last refresh (cheap). |
//! | [`get_tdx_mr_refresh`] | Yes | Reading a register whose current hardware value is needed — e.g. immediately after [`extend_tdx_mr`]. The refresh and the register read are performed atomically under the write lock. |
//! | [`refresh_tdx_report`] | Yes | Regenerating the report with a custom `report_data` blob (used by the `TDX_CMD_GET_REPORT0` ioctl and the quote path) without reading a specific register. |
//! | [`extend_tdx_mr`] | Yes | Extending an RTMR. After extending, the cached report is stale; use [`get_tdx_mr_refresh`] (or [`refresh_tdx_report`] followed by [`get_tdx_mr`]) to observe the updated value. |
//!
//! For the TDX architecture specification see Intel's
//! [TDX Module Specification](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-trust-domain-extensions.html).

use alloc::sync::Arc;
use core::{
    mem::{offset_of, size_of},
    time::Duration,
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use device_id::{DeviceId, MinorId};
use ostd::{
    const_assert,
    mm::{FrameAllocOptions, HasPaddr, HasSize, PAGE_SIZE, USegment, VmIo, dma::DmaCoherent},
    sync::{RwMutexWriteGuard, WaitQueue},
};
use spin::Once;
use tdx_guest::{
    SHARED_MASK,
    tdcall::{self, TdCallError},
    tdvmcall::{self, TdVmcallError},
};

use crate::{
    device::{Device, DeviceType, registry::char::register},
    events::IoEvents,
    fs::{
        file::{FileIo, StatusFlags},
        vfs::inode::InodeIo,
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

                    let report = TDX_REPORT
                        .get()
                        .ok_or_else(|| {
                            Error::with_message(Errno::ENODEV, "TDX report not initialized")
                        })?
                        .write();
                    refresh_tdx_report_locked(&report, Some(inblob.as_bytes()))?;
                    let outblob_ptr = field_ptr!(&data_ptr, TdxReportRequest, tdx_report);
                    outblob_ptr.copy_from(&SafePtr::new(&*report, 0))?;

                    Ok(0)
                })
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown"),
        })
    }
}

static TDX_REPORT: Once<RwMutex<USegment>> = Once::new();

/// Runtime Measurement Register (RTMR) index.
///
/// RTMRs are the only measurement registers that can be extended at runtime
/// via [`extend_tdx_mr`].
#[derive(Clone, Copy, Debug)]
pub enum Rtmr {
    Rtmr0,
    Rtmr1,
    Rtmr2,
    Rtmr3,
}

/// TDX measurement register.
#[derive(Clone, Copy, Debug)]
pub enum MeasurementReg {
    MrConfigId,
    MrOwner,
    MrOwnerConfig,
    MrTd,
    Rtmr(Rtmr),
}

/// Gets the TDX quote given the specified data in `inblob`.
pub fn tdx_get_quote(inblob: &[u8]) -> Result<Box<[u8]>> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    const GET_QUOTE_BUF_SIZE: usize = 8 * 1024;

    let buf = DmaCoherent::alloc(GET_QUOTE_BUF_SIZE.div_ceil(PAGE_SIZE), false)?;
    let header_ptr: SafePtr<TdxQuoteHdr, _, _> = SafePtr::new(&buf, 0);
    let payload_ptr: SafePtr<TdReport, _, _> = SafePtr::new(&buf, size_of::<TdxQuoteHdr>());

    field_ptr!(&header_ptr, TdxQuoteHdr, version).write(&1u64)?;
    field_ptr!(&header_ptr, TdxQuoteHdr, status).write(&0u64)?;
    field_ptr!(&header_ptr, TdxQuoteHdr, in_len).write(&(size_of::<TdReport>() as u32))?;
    field_ptr!(&header_ptr, TdxQuoteHdr, out_len).write(&0u32)?;

    let report = TDX_REPORT
        .get()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "TDX report not initialized"))?
        .write();
    refresh_tdx_report_locked(&report, Some(inblob))?;
    payload_ptr.copy_from(&SafePtr::new(&*report, 0))?;
    drop(report);

    // FIXME: The `get_quote` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    tdvmcall::get_quote((buf.paddr() as u64) | SHARED_MASK, buf.size() as u64)?;

    // Poll for the quote to be ready.
    let status_ptr = field_ptr!(&header_ptr, TdxQuoteHdr, status);
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
    let out_len = field_ptr!(&header_ptr, TdxQuoteHdr, out_len).read()?;
    let mut outblob = vec![0u8; out_len as usize].into_boxed_slice();
    payload_ptr.cast().read_slice(outblob.as_mut())?;
    Ok(outblob)
}

/// Refreshes the global TDX report cache, issuing a `TDG.MR.REPORT` TDCALL.
///
/// If `inblob` is `Some`, it is written into the report's `REPORTDATA` field
/// before the TDCALL; otherwise the field already present in the cached report
/// is reused.
///
/// Most callers that only need to read a measurement register after an extend
/// should use [`get_tdx_mr_refresh`] instead, which combines the refresh and
/// the register read atomically.
pub fn refresh_tdx_report(inblob: Option<&[u8]>) -> Result<()> {
    let report = TDX_REPORT
        .get()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "TDX report not initialized"))?
        .write();
    refresh_tdx_report_locked(&report, inblob)
}

pub const SHA384_DIGEST_SIZE: usize = 48;

/// Gets the measurement register value from the cached TDX report.
///
/// This is a cheap read — no TDCALL is issued. The value reflects the state of
/// the report at the time of the last [`refresh_tdx_report`] or
/// [`get_tdx_mr_refresh`] call. If the register may have been updated by a
/// recent [`extend_tdx_mr`], use [`get_tdx_mr_refresh`] instead to obtain the
/// current hardware value.
pub fn get_tdx_mr(reg: MeasurementReg) -> Result<[u8; SHA384_DIGEST_SIZE]> {
    let report = TDX_REPORT
        .get()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "TDX report not initialized"))?
        .read();

    let mut blob = [0u8; SHA384_DIGEST_SIZE];
    report
        .read_bytes(TdReport::mr_offset(reg), blob.as_mut())
        .unwrap();
    Ok(blob)
}

/// Refreshes the TDX report and returns the requested measurement register value.
///
/// Issues a `TDG.MR.REPORT` TDCALL and then reads `reg` from the newly
/// generated report. The refresh and the read are performed atomically under
/// the write lock, so the returned value is guaranteed to reflect the hardware
/// state at the time of the call.
///
/// Use this function after [`extend_tdx_mr`] to observe the updated RTMR
/// value. If no extend has occurred and the cached report is still current,
/// the cheaper [`get_tdx_mr`] can be used instead.
pub fn get_tdx_mr_refresh(reg: MeasurementReg) -> Result<[u8; SHA384_DIGEST_SIZE]> {
    let report = TDX_REPORT
        .get()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "TDX report not initialized"))?
        .write();

    refresh_tdx_report_locked(&report, None)?;

    let mut blob = [0u8; SHA384_DIGEST_SIZE];
    report
        .read_bytes(TdReport::mr_offset(reg), blob.as_mut())
        .unwrap();
    Ok(blob)
}

/// Extends an RTMR with the given SHA-384 digest, issuing a
/// `TDG.MR.RTMR.EXTEND` TDCALL.
///
/// After this call the cached TDX report is stale with respect to the extended
/// register. To read the updated value, use [`get_tdx_mr_refresh`], which
/// atomically regenerates the report and reads the register. Alternatively,
/// call [`refresh_tdx_report`] and then [`get_tdx_mr`] if you need to refresh
/// once and read multiple registers.
pub fn extend_tdx_mr(reg: Rtmr, data: &[u8; SHA384_DIGEST_SIZE]) -> Result<()> {
    let index = match reg {
        Rtmr::Rtmr0 => 0,
        Rtmr::Rtmr1 => 1,
        Rtmr::Rtmr2 => 2,
        Rtmr::Rtmr3 => 3,
    };

    // `TDG.MR.RTMR.EXTEND` requires a 64B-aligned physical address; a
    // page-aligned frame meets that requirement. RTMR extension is infrequent,
    // so the per-call allocation overhead is negligible.
    let buf: USegment = FrameAllocOptions::new().alloc_segment(1)?.into();
    buf.write_bytes(0, data).unwrap();

    tdcall::extend_rtmr(buf.paddr() as u64, index)?;
    Ok(())
}

pub(super) fn init() -> Result<()> {
    TDX_REPORT.call_once(|| {
        let report = FrameAllocOptions::new()
            .alloc_segment(size_of::<TdReport>().div_ceil(PAGE_SIZE))
            .unwrap()
            .into();
        RwMutex::new(report)
    });
    refresh_tdx_report(None)?;
    register(TdxGuest::new())?;
    Ok(())
}

fn refresh_tdx_report_locked(
    report: &RwMutexWriteGuard<'_, USegment>,
    inblob: Option<&[u8]>,
) -> Result<()> {
    if let Some(inblob) = inblob {
        if inblob.len() != size_of::<ReportData>() {
            return_errno_with_message!(Errno::EINVAL, "Invalid inblob length");
        }

        // Use `inblob` as the data associated with the report.
        report
            .write_bytes(TdReport::report_data_offset(), inblob)
            .unwrap();
    }

    // From TDX Module Specification, the report structure returned by TDX Module
    // places the report data at offset 128, so using the same offset keeps the
    // memory layout consistent with the TDX Modules's output format. And we can
    // directly call `get_report` on the existing report structure without needing
    // to rewrite the report data.
    let report_data_ptr = report.paddr() + TdReport::report_data_offset();

    // FIXME: The `get_report` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    tdcall::get_report(report.paddr() as u64, report_data_ptr as u64)?;
    Ok(())
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct TdxReportRequest {
    report_data: ReportData,
    tdx_report: TdReport,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct ReportData {
    data: [u8; 64],
}

/// TDX Report structure (`TDREPORT_STRUCT`) as defined in the Intel TDX Module Specification.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct TdReport {
    report_mac: ReportMac,
    _reserved: [u8; 256],
    td_info: TdInfo,
}
const_assert!(size_of::<TdReport>() == 1024);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct ReportMac {
    _reserved1: [u8; 128],
    report_data: ReportData,
    _reserved2: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct TdInfo {
    attributes: [u8; 8],
    xfam: [u8; 8],
    mrtd: [u8; 48],
    mrconfigid: [u8; 48],
    mrowner: [u8; 48],
    mrownerconfig: [u8; 48],
    rtmr0: [u8; 48],
    rtmr1: [u8; 48],
    rtmr2: [u8; 48],
    rtmr3: [u8; 48],
    servtd_hash: [u8; 48],
    extension: [u8; 64],
}

impl TdReport {
    const fn report_data_offset() -> usize {
        offset_of!(TdReport, report_mac) + offset_of!(ReportMac, report_data)
    }

    const fn mr_offset(reg: MeasurementReg) -> usize {
        offset_of!(TdReport, td_info)
            + match reg {
                MeasurementReg::MrConfigId => offset_of!(TdInfo, mrconfigid),
                MeasurementReg::MrOwner => offset_of!(TdInfo, mrowner),
                MeasurementReg::MrOwnerConfig => offset_of!(TdInfo, mrownerconfig),
                MeasurementReg::MrTd => offset_of!(TdInfo, mrtd),
                MeasurementReg::Rtmr(Rtmr::Rtmr0) => offset_of!(TdInfo, rtmr0),
                MeasurementReg::Rtmr(Rtmr::Rtmr1) => offset_of!(TdInfo, rtmr1),
                MeasurementReg::Rtmr(Rtmr::Rtmr2) => offset_of!(TdInfo, rtmr2),
                MeasurementReg::Rtmr(Rtmr::Rtmr3) => offset_of!(TdInfo, rtmr3),
            }
    }
}

mod ioctl_defs {
    use super::TdxReportRequest;
    use crate::util::ioctl::{InOutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/tdx-guest.h#L40>

    pub(super) type GetTdxReport =
        ioc!(TDX_CMD_GET_REPORT0, b'T', 0x01, InOutData<TdxReportRequest>);
}
