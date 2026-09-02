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

/// A wire-format message body whose fields are interpreted by the request handler.
pub(crate) trait SegmentBody: Pod + From<Self::CLegacyType> {
    // The actual message body should be `Self`,
    // but older versions of Linux use a legacy type (usually `CRtGenMsg` here).
    // Additionally, some software, like iproute2, also uses this legacy type.
    // Therefore, we need to handle both cases.
    // Reference: <https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393>.
    // FIXME: Verify whether the legacy type includes any types other than `CRtGenMsg`.
    type CLegacyType: Pod = Self;

    /// Reads the segment body from the `reader`.
    ///
    /// Returns the body and the remaining payload length after body alignment padding,
    /// excluding trailing message alignment padding.
    fn read_from(
        header: &CMsgSegHdr,
        reader: &mut dyn MultiRead,
        strict_check: bool,
    ) -> Result<ContinueRead<(Self, usize)>> {
        let mut remaining_len = header.calc_payload_len(reader)?;

        // Read the body.
        //
        // Legacy `rtgenmsg` bodies are accepted only without strict checking
        // for compatibility with older iproute2 versions.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L2430>.
        if strict_check && remaining_len < size_of::<Self>() {
            reader.skip_some(remaining_len);
            return Ok(ContinueRead::skipped_with_error(
                Errno::EINVAL,
                "the message body is too short",
            ));
        }

        let (body, padding_len) = if remaining_len >= size_of::<Self>() {
            let body = reader.read_val_opt::<Self>()?.unwrap();
            remaining_len -= size_of_val(&body);

            (body, Self::padding_len())
        } else if remaining_len >= size_of::<Self::CLegacyType>() {
            let legacy = reader.read_val_opt::<Self::CLegacyType>()?.unwrap();
            remaining_len -= size_of_val(&legacy);

            (Self::from(legacy), Self::legacy_padding_len())
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

        Ok(ContinueRead::Parsed((body, remaining_len)))
    }

    /// Writes the segment body to the `writer`.
    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        // Write the body.
        writer.write_val_trunc(self)?;

        // Skip the padding bytes.
        let padding_len = Self::padding_len();
        writer.skip_some(padding_len);

        Ok(())
    }

    fn padding_len() -> usize {
        let payload_len = size_of::<Self>();
        payload_len.align_up(NLMSG_ALIGN) - payload_len
    }

    fn legacy_padding_len() -> usize {
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
