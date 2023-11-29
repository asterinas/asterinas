use super::*;
use crate::events::IoEvents;
use crate::fs::inode_handle::FileIo;
use crate::fs::utils::IoctlCmd;
use crate::process::signal::Poller;
use crate::util::{read_val_from_user, write_val_to_user};
use tdx_guest::tdcall::get_report;

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
        DeviceId::new(10, 0)
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
            IoctlCmd::TDXGETREPORT => {
                let tdx_report: TdxReportRequest = read_val_from_user(arg)?;
                get_report(&tdx_report.tdreport, &tdx_report.reportdata).unwrap();
                write_val_to_user(arg, &tdx_report)?;
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
