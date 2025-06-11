// SPDX-License-Identifier: MPL-2.0

use super::{header::CMsgSegHdr, SegmentBody};
use crate::{
    net::socket::netlink::message::attr::Attribute,
    prelude::*,
    util::{MultiRead, MultiWrite},
};

#[derive(Debug)]
pub struct SegmentCommon<Body, Attr> {
    header: CMsgSegHdr,
    body: Body,
    attrs: Vec<Attr>,
}

impl<Body, Attr> SegmentCommon<Body, Attr> {
    pub const HEADER_LEN: usize = size_of::<CMsgSegHdr>();

    pub fn header(&self) -> &CMsgSegHdr {
        &self.header
    }

    pub fn header_mut(&mut self) -> &mut CMsgSegHdr {
        &mut self.header
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    pub fn attrs(&self) -> &Vec<Attr> {
        &self.attrs
    }
}

impl<Body: SegmentBody, Attr: Attribute> SegmentCommon<Body, Attr> {
    pub const BODY_LEN: usize = size_of::<Body::CType>();

    pub fn new(header: CMsgSegHdr, body: Body, attrs: Vec<Attr>) -> Self {
        let mut res = Self {
            header,
            body,
            attrs,
        };
        res.header.len = res.total_len() as u32;
        res
    }

    pub fn read_from(header: CMsgSegHdr, reader: &mut dyn MultiRead) -> Result<Self>
    where
        Error: From<<Body::CType as TryInto<Body>>::Error>,
    {
        let (body, remain_len) = Body::read_from(&header, reader)?;
        let attrs = Attr::read_all_from(reader, remain_len)?;

        Ok(Self {
            header,
            body,
            attrs,
        })
    }

    pub fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        writer.write_val_trunc(&self.header)?;

        self.body.write_to(writer)?;
        for attr in self.attrs.iter() {
            attr.write_to(writer)?;
        }

        Ok(())
    }

    pub fn total_len(&self) -> usize {
        Self::HEADER_LEN + Self::BODY_LEN + self.attrs_len()
    }
}

impl<Body, Attr: Attribute> SegmentCommon<Body, Attr> {
    pub fn attrs_len(&self) -> usize {
        self.attrs
            .iter()
            .map(|attr| attr.total_len_with_padding())
            .sum()
    }
}
