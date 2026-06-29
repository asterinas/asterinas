// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::message::{Attribute, CAttrHeader, ContinueRead},
    prelude::*,
    util::MultiRead,
};

/// Route-related attributes.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L394>.
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum RouteAttrClass {
    UNSPEC    = 0,
    DST       = 1,
    SRC       = 2,
    IIF       = 3,
    OIF       = 4,
    GATEWAY   = 5,
    PRIORITY  = 6,
    PREFSRC   = 7,
}

#[derive(Clone, Debug)]
pub enum RouteAttr {
    Dst([u8; 4]),
    Gateway([u8; 4]),
    Oif(u32),
}

impl RouteAttr {
    fn class(&self) -> RouteAttrClass {
        match self {
            RouteAttr::Dst(_)     => RouteAttrClass::DST,
            RouteAttr::Gateway(_) => RouteAttrClass::GATEWAY,
            RouteAttr::Oif(_)     => RouteAttrClass::OIF,
        }
    }
}

impl Attribute for RouteAttr {
    fn type_(&self) -> u16 {
        self.class() as u16
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            RouteAttr::Dst(addr) | RouteAttr::Gateway(addr) => addr,
            RouteAttr::Oif(_) => unreachable!("Oif is not written to user space"),
        }
    }

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<ContinueRead<Self>>
    where
        Self: Sized,
    {
        let attr_type = RouteAttrClass::try_from(header.type_()).ok();
        let payload_len = header.payload_len();

        match attr_type {
            Some(RouteAttrClass::GATEWAY) if payload_len == 4 => {
                let octets: [u8; 4] = reader.read_val_opt()?.unwrap();
                Ok(ContinueRead::Parsed(RouteAttr::Gateway(octets)))
            }
            Some(RouteAttrClass::DST) if payload_len == 4 => {
                let octets: [u8; 4] = reader.read_val_opt()?.unwrap();
                Ok(ContinueRead::Parsed(RouteAttr::Dst(octets)))
            }
            Some(RouteAttrClass::OIF) if payload_len == 4 => {
                let index: u32 = reader.read_val_opt()?.unwrap();
                Ok(ContinueRead::Parsed(RouteAttr::Oif(index)))
            }
            _ => {
                reader.skip_some(payload_len);
                Ok(ContinueRead::Skipped)
            }
        }
    }
}
