// SPDX-License-Identifier: MPL-2.0

use tdx_guest::tdcall::TdCallError;

use super::*;
use crate::{
    error::Error,
    events::IoEvents,
    fs::{inode_handle::FileIo, utils::IoctlCmd},
    process::signal::Poller,
};

const TDX_REPORTDATA_LEN: usize = 64;
const TDX_REPORT_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct TdxReportRequest {
    reportdata: [u8; TDX_REPORTDATA_LEN],
    tdreport: [u8; TDX_REPORT_LEN],
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

impl FileIo for TdxGuest {
    fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Read operation not supported")
    }

    fn write(&self, _buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Write operation not supported")
    }

    fn ioctl(&self, cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TDXGETREPORT => {
                todo!()
            }
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
