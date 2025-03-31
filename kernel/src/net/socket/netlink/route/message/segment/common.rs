// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::{header::CMessageSegmentHeader, SegmentBody};
use crate::{
    net::socket::netlink::route::message::{attr::Attribute, NLMSG_ALIGN},
    prelude::*,
};

#[derive(Debug)]
pub struct SegmentCommon<Body, Attr> {
    header: CMessageSegmentHeader,
    body: Body,
    attrs: Vec<Attr>,
}

impl<Body, Attr> SegmentCommon<Body, Attr> {
    pub const HEADER_LEN: usize = size_of::<CMessageSegmentHeader>();

    pub fn header(&self) -> &CMessageSegmentHeader {
        &self.header
    }

    pub fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
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

    pub fn new(header: CMessageSegmentHeader, body: Body, attrs: Vec<Attr>) -> Self {
        let mut res = Self {
            header,
            body,
            attrs,
        };
        res.header.len = res.total_len() as u32;
        res
    }

    pub fn read_from(header: CMessageSegmentHeader, reader: &mut VmReader) -> Result<Self>
    where
        Error: From<<Body::CType as TryInto<Body>>::Error>,
    {
        let (body, body_len) = Body::read_body_from_user(&header, reader)?;

        let attrs = {
            let attrs_len = (header.len as usize - size_of::<CMessageSegmentHeader>() - body_len)
                .align_down(NLMSG_ALIGN);
            Attr::read_all_from(reader, attrs_len)?
        };

        Ok(Self {
            header,
            body,
            attrs,
        })
    }

    pub fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        writer.write_val(&self.header)?;
        self.body.write_body_to_user(writer)?;
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
