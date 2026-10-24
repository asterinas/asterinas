// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use header::CMsgSegHdr;

use super::{ContinueRead, NLMSG_ALIGN};
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) mod ack;
pub(super) mod common;
pub(super) mod header;

pub(crate) trait SegmentBody: Sized + Clone + Copy {
    // The actual message body should be `Self::CType`,
    // but older versions of Linux use a legacy type (usually `CRtGenMsg` here).
    // Additionally, some software, like iproute2, also uses this legacy type.
    // Therefore, we need to handle both cases.
    // Reference: <https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393>.
    // FIXME: Verify whether the legacy type includes any types other than `CRtGenMsg`.
    type CLegacyType: Pod = Self::CType;
    type CType: Pod + From<Self::CLegacyType> + TryInto<Self> + From<Self>;

    /// Validates the raw request body when strict checking is enabled.
    fn validate_strict_request_body(_header: &CMsgSegHdr, _body: &Self::CType) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the segment type is not supported");
    }

    /// Normalizes ignored fields in legacy requests.
    fn normalize_legacy_request_body(_header: &CMsgSegHdr, _body: &mut Self::CType) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the segment type is not supported");
    }

    /// Reads the segment body from the `reader`.
    ///
    /// Returns the body and the remaining payload length after body alignment padding,
    /// excluding trailing message alignment padding.
    fn read_from(
        header: &CMsgSegHdr,
        reader: &mut dyn MultiRead,
        strict_check: bool,
    ) -> Result<ContinueRead<(Self, usize)>>
    where
        Error: From<<Self::CType as TryInto<Self>>::Error>,
    {
        let mut remaining_len = header.calc_payload_len(reader)?;

        // Read the body.
        //
        // Legacy `rtgenmsg` bodies are accepted only without strict checking
        // for compatibility with older iproute2 versions.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L2430>.
        if strict_check && remaining_len < size_of::<Self::CType>() {
            reader.skip_some(remaining_len);
            return Ok(ContinueRead::skipped_with_error(
                Errno::EINVAL,
                "the message body is too short",
            ));
        }

        let (mut c_type, padding_len) = if remaining_len >= size_of::<Self::CType>() {
            let c_type = reader.read_val_opt::<Self::CType>()?.unwrap();
            remaining_len -= size_of_val(&c_type);

            (c_type, Self::padding_len())
        } else if remaining_len >= size_of::<Self::CLegacyType>() {
            let legacy = reader.read_val_opt::<Self::CLegacyType>()?.unwrap();
            remaining_len -= size_of_val(&legacy);

            (Self::CType::from(legacy), Self::lecacy_padding_len())
        } else {
            reader.skip_some(remaining_len);
            return Ok(ContinueRead::skipped_with_error(
                Errno::EINVAL,
                "the message length is too small",
            ));
        };

        // Skip the body alignment padding.
        let padding_len = padding_len.min(remaining_len);
        reader.skip_some(padding_len);
        remaining_len -= padding_len;

        let result = if strict_check {
            Self::validate_strict_request_body(header, &c_type)
        } else {
            Self::normalize_legacy_request_body(header, &mut c_type)
        };
        if let Err(err) = result {
            reader.skip_some(remaining_len);
            return Ok(ContinueRead::SkippedErr(err));
        }
        match c_type.try_into() {
            Ok(body) => Ok(ContinueRead::Parsed((body, remaining_len))),
            Err(err) => {
                reader.skip_some(remaining_len);
                Ok(ContinueRead::SkippedErr(err.into()))
            }
        }
    }

    /// Writes the segment body to the `writer`.
    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        // Write the body.
        let c_body = Self::CType::from(*self);
        writer.write_val_trunc(&c_body)?;

        // Skip the padding bytes.
        let padding_len = Self::padding_len();
        writer.skip_some(padding_len);

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

#[expect(clippy::upper_case_acronyms)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, TryFromInt)]
pub(in crate::net::socket::netlink) enum CSegmentType {
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
