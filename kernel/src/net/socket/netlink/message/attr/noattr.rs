// SPDX-License-Identifier: MPL-2.0

use super::{Attribute, CAttrHeader};
use crate::{net::socket::netlink::message::ContinueRead, prelude::*, util::MultiRead};

/// A special type indicates that a segment cannot have attributes.
#[derive(Debug)]
pub enum NoAttr {}

impl Attribute for NoAttr {
    fn type_(&self) -> u16 {
        match *self {}
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match *self {}
    }

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<ContinueRead<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();
        reader.skip_some(payload_len);

        Ok(ContinueRead::Skipped)
    }

    fn read_all_from(
        reader: &mut dyn MultiRead,
        total_len: usize,
    ) -> Result<ContinueRead<Vec<Self>>>
    where
        Self: Sized,
    {
        reader.skip_some(total_len);

        Ok(ContinueRead::Skipped)
    }
}
