// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use header::CMsgSegHdr;

use super::NLMSG_ALIGN;
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub mod ack;
pub mod common;
pub mod header;

pub trait SegmentBody: Sized + Clone + Copy {
    // The actual message body should be `Self::CType`,
    // but older versions of Linux use a legacy type (usually `CRtGenMsg` here).
    // Additionally, some software, like iproute2, also uses this legacy type.
    // Therefore, we need to handle both cases.
    // Reference: <https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393>.
    // FIXME: Verify whether the legacy type includes any types other than `CRtGenMsg`.
    type CLegacyType: Pod = Self::CType;
    type CType: Pod + From<Self::CLegacyType> + TryInto<Self> + From<Self>;

    fn read_from(header: &CMsgSegHdr, reader: &mut dyn MultiRead) -> Result<(Self, usize)>
    where
        Error: From<<Self::CType as TryInto<Self>>::Error>,
    {
        let max_len = header.len as usize - size_of_val(header);

        let (c_type, read_size) = if max_len >= size_of::<Self::CType>() {
            let c_type = reader.read_val::<Self::CType>()?;

            let skip_len = Self::padding_len().min(reader.sum_lens());
            reader.skip(skip_len);

            (c_type, size_of::<Self::CType>() + skip_len)
        } else if max_len >= size_of::<Self::CLegacyType>() {
            let legacy = reader.read_val::<Self::CLegacyType>()?;

            let skip_len = Self::lecacy_padding_len().min(reader.sum_lens());
            reader.skip(skip_len);

            (
                Self::CType::from(legacy),
                size_of::<Self::CLegacyType>() + skip_len,
            )
        } else {
            return_errno_with_message!(Errno::EINVAL, "the message length is too small");
        };

        let body = c_type.try_into()?;
        Ok((body, read_size))
    }

    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let c_body = Self::CType::from(*self);
        writer.write_val(&c_body)?;
        let padding_len = Self::padding_len();
        writer.skip(padding_len.min(writer.sum_lens()));
        Ok(())
    }

    fn padding_len() -> usize {
        let payload_len = size_of::<Self::CType>();
        payload_len.align_up(NLMSG_ALIGN) - payload_len
    }

    fn lecacy_padding_len() -> usize {
        let payload_len = size_of::<Self::CLegacyType>();
        payload_len.align_up(NLMSG_ALIGN) - payload_len
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
pub enum CSegmentType {
    // Standard netlink message types
    NOOP = 1,
    ERROR = 2,
    DONE = 3,
    OVERRUN = 4,

    // protocol-level types
    NEWLINK = 16,
    DELLINK = 17,
    GETLINK = 18,
    SETLINK = 19,

    NEWADDR = 20,
    DELADDR = 21,
    GETADDR = 22,

    NEWROUTE = 24,
    DELROUTE = 25,
    GETROUTE = 26,
    // TODO: The list is not exhaustive.
}
