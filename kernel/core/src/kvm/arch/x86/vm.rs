// SPDX-License-Identifier: MPL-2.0

use super::ioctl::SetTssAddr;
use crate::{dispatch_ioctl, kvm::vm::Vm, prelude::*, util::ioctl::RawIoctl};

impl Vm {
    pub(crate) fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            SetTssAddr => {
                Ok(0)
            }
            _ => {
                return_errno_with_message!(Errno::ENOTTY, "unsupported VM ioctl")
            }
        })
    }
}
