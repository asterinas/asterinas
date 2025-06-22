// SPDX-License-Identifier: MPL-2.0

use super::{Attribute, CAttrHeader};
use crate::{prelude::*, util::MultiRead};

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

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<Option<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();
        reader.skip_some(payload_len);

        Ok(None)
    }

    fn read_all_from(_reader: &mut dyn MultiRead, _total_len: usize) -> Result<Vec<Self>>
    where
        Self: Sized,
    {
        Ok(Vec::new())
    }
}
