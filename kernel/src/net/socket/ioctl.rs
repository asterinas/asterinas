// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroI32;

use crate::{
    net::iface::iter_all_ifaces,
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

const IFNAMSIZ: usize = 16;
const IFREQ_DATA_SIZE: usize = 24;

mod ioctl_defs {
    use super::CIfReq;
    use crate::util::ioctl::{InOutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/sockios.h>
    pub(super) type GetIfName  = ioc!(SIOCGIFNAME,  0x8910, InOutData<CIfReq>);
    pub(super) type GetIfIndex = ioc!(SIOCGIFINDEX, 0x8933, InOutData<CIfReq>);
}

pub(super) fn socket_ioctl(raw_ioctl: RawIoctl) -> Result<i32> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ GetIfIndex => {
            let mut ifreq = cmd.read()?;
            let iface = find_iface_by_name(ifreq.name_bytes())?;
            ifreq.set_index(iface.index())?;
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfName => {
            let mut ifreq = cmd.read()?;
            let index = ifreq.index()?;
            let iface = iter_all_ifaces()
                .find(|iface| iface.index() == index.get() as u32)
                .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))?;
            ifreq.set_name(iface.name())?;
            cmd.write(&ifreq)?;
            Ok(0)
        }
        _ => return_errno_with_message!(Errno::ENOTTY, "the socket ioctl command is unknown"),
    })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CIfReq {
    name: [u8; IFNAMSIZ],
    data: [u8; IFREQ_DATA_SIZE],
}

impl CIfReq {
    fn name_bytes(&self) -> &[u8] {
        let name_len = self
            .name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(IFNAMSIZ);
        &self.name[..name_len]
    }

    fn set_name(&mut self, name: &CStr) -> Result<()> {
        let name = name.to_bytes_with_nul();
        if name.len() > IFNAMSIZ {
            return_errno_with_message!(Errno::ERANGE, "the interface name is too long");
        }

        self.name = [0; IFNAMSIZ];
        self.name[..name.len()].copy_from_slice(name);
        Ok(())
    }

    fn index(&self) -> Result<NonZeroI32> {
        let index = i32::from_ne_bytes(self.data[..size_of::<i32>()].try_into().unwrap());
        NonZeroI32::new(index)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))
    }

    fn set_index(&mut self, index: u32) -> Result<()> {
        let index = i32::try_from(index).map_err(|_| {
            Error::with_message(Errno::EOVERFLOW, "the interface index is too large")
        })?;
        self.data[..size_of::<i32>()].copy_from_slice(&index.to_ne_bytes());
        Ok(())
    }
}

fn find_iface_by_name(name: &[u8]) -> Result<&'static Arc<crate::net::iface::Iface>> {
    iter_all_ifaces()
        .find(|iface| iface.name().to_bytes() == name)
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))
}
