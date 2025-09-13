// SPDX-License-Identifier: MPL-2.0

use super::{header::CMsgSegHdr, SegmentBody};
use crate::{
    net::socket::netlink::message::{attr::Attribute, ContinueRead},
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

    pub fn read_from(header: &CMsgSegHdr, reader: &mut dyn MultiRead) -> Result<ContinueRead<Self>>
    where
        Error: From<<Body::CType as TryInto<Body>>::Error>,
    {
        let (body, remain_len) = match Body::read_from(header, reader)? {
            ContinueRead::Parsed(parsed) => parsed,
            ContinueRead::Skipped => return Ok(ContinueRead::Skipped),
            ContinueRead::SkippedErr(err) => return Ok(ContinueRead::SkippedErr(err)),
        };

        let attrs = match Attr::read_all_from(reader, remain_len)? {
            ContinueRead::Parsed(attrs) => attrs,
            ContinueRead::Skipped => Vec::new(),
            ContinueRead::SkippedErr(err) => return Ok(ContinueRead::SkippedErr(err)),
        };

        Ok(ContinueRead::Parsed(Self {
            header: *header,
            body,
            attrs,
        }))
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
