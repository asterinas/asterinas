// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::message::{Attribute, CAttrHeader, ContinueRead},
    prelude::*,
    util::MultiRead,
};

/// Route attributes.
#[expect(clippy::upper_case_acronyms)]
#[expect(non_camel_case_types)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum RouteAttrClass {
    UNSPEC = 0,
    DST = 1,
    SRC = 2,
    IIF = 3,
    OIF = 4,
    GATEWAY = 5,
    PRIORITY = 6,
    PREFSRC = 7,
    METRICS = 8,
    MULTIPATH = 9,
    PROTOINFO = 10,
    FLOW = 11,
    CACHEINFO = 12,
    SESSION = 13,
    MP_ALGO = 14,
    TABLE = 15,
    MARK = 16,
    MFC_STATS = 17,
    VIA = 18,
    NEWDST = 19,
    PREF = 20,
    ENCAP_TYPE = 21,
    ENCAP = 22,
    EXPIRES = 23,
    PAD = 24,
    UID = 25,
    TTL_PROPAGATE = 26,
    IP_PROTO = 27,
    SPORT = 28,
    DPORT = 29,
    NH_ID = 30,
}

/// Supported route attributes.
#[derive(Debug)]
pub enum RouteAttr {
    Dst(Vec<u8>),
    Gateway(Vec<u8>),
    Iif(u32),
    Oif(u32),
    PrefSrc(Vec<u8>),
    Priority(u32),
    Src(Vec<u8>),
    Table(u32),
}

impl RouteAttr {
    fn class(&self) -> RouteAttrClass {
        match self {
            RouteAttr::Dst(_) => RouteAttrClass::DST,
            RouteAttr::Gateway(_) => RouteAttrClass::GATEWAY,
            RouteAttr::Iif(_) => RouteAttrClass::IIF,
            RouteAttr::Oif(_) => RouteAttrClass::OIF,
            RouteAttr::PrefSrc(_) => RouteAttrClass::PREFSRC,
            RouteAttr::Priority(_) => RouteAttrClass::PRIORITY,
            RouteAttr::Src(_) => RouteAttrClass::SRC,
            RouteAttr::Table(_) => RouteAttrClass::TABLE,
        }
    }
}

impl Attribute for RouteAttr {
    fn type_(&self) -> u16 {
        self.class() as u16
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            RouteAttr::Dst(dst) => dst,
            RouteAttr::Gateway(gateway) => gateway,
            RouteAttr::Iif(iif) => iif.as_bytes(),
            RouteAttr::Oif(oif) => oif.as_bytes(),
            RouteAttr::PrefSrc(pref_src) => pref_src,
            RouteAttr::Priority(priority) => priority.as_bytes(),
            RouteAttr::Src(src) => src,
            RouteAttr::Table(table) => table.as_bytes(),
        }
    }

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<ContinueRead<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();
        let Ok(class) = RouteAttrClass::try_from(header.type_()) else {
            reader.skip_some(payload_len);
            return Ok(ContinueRead::Skipped);
        };

        let attr = match (class, payload_len) {
            (RouteAttrClass::DST, 4 | 16) => Self::Dst(read_addr(reader, payload_len)?),
            (RouteAttrClass::GATEWAY, 4 | 16) => Self::Gateway(read_addr(reader, payload_len)?),
            (RouteAttrClass::IIF, 4) => Self::Iif(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::OIF, 4) => Self::Oif(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::PREFSRC, 4 | 16) => Self::PrefSrc(read_addr(reader, payload_len)?),
            (RouteAttrClass::PRIORITY, 4) => Self::Priority(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::SRC, 4 | 16) => Self::Src(read_addr(reader, payload_len)?),
            (RouteAttrClass::TABLE, 4) => Self::Table(reader.read_val_opt::<u32>()?.unwrap()),

            (
                RouteAttrClass::DST
                | RouteAttrClass::GATEWAY
                | RouteAttrClass::IIF
                | RouteAttrClass::OIF
                | RouteAttrClass::PREFSRC
                | RouteAttrClass::PRIORITY
                | RouteAttrClass::SRC
                | RouteAttrClass::TABLE,
                _,
            ) => {
                reader.skip_some(payload_len);
                return Ok(ContinueRead::skipped_with_error(
                    Errno::EINVAL,
                    "the route attribute length is invalid",
                ));
            }

            (
                RouteAttrClass::METRICS
                | RouteAttrClass::MULTIPATH
                | RouteAttrClass::PROTOINFO
                | RouteAttrClass::FLOW
                | RouteAttrClass::CACHEINFO
                | RouteAttrClass::SESSION
                | RouteAttrClass::MP_ALGO
                | RouteAttrClass::VIA
                | RouteAttrClass::NEWDST
                | RouteAttrClass::PREF
                | RouteAttrClass::EXPIRES
                | RouteAttrClass::MARK
                | RouteAttrClass::MFC_STATS
                | RouteAttrClass::ENCAP_TYPE
                | RouteAttrClass::ENCAP
                | RouteAttrClass::UID
                | RouteAttrClass::TTL_PROPAGATE
                | RouteAttrClass::IP_PROTO
                | RouteAttrClass::SPORT
                | RouteAttrClass::DPORT
                | RouteAttrClass::NH_ID,
                _,
            ) => {
                reader.skip_some(payload_len);
                return Ok(ContinueRead::skipped_with_error(
                    Errno::EOPNOTSUPP,
                    "the route attribute is not supported",
                ));
            }

            (RouteAttrClass::PAD, _) => {
                reader.skip_some(payload_len);
                return Ok(ContinueRead::Skipped);
            }

            (_, _) => {
                reader.skip_some(payload_len);
                return Ok(ContinueRead::Skipped);
            }
        };

        Ok(ContinueRead::Parsed(attr))
    }
}

fn read_addr(reader: &mut dyn MultiRead, len: usize) -> Result<Vec<u8>> {
    let mut addr = vec![0; len];
    let read_len = reader.read(&mut VmWriter::from(addr.as_mut_slice()))?;
    if read_len != len {
        return_errno_with_message!(Errno::EINVAL, "the route attribute length is invalid");
    }
    Ok(addr)
}
