// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SocketAddr;
use crate::{net::socket::unix::UnixControlMessage, prelude::*, util::net::CSocketOptionLevel};

/// Message header used for sendmsg/recvmsg.
#[derive(Debug)]
pub struct MessageHeader {
    pub(in crate::net) addr: Option<SocketAddr>,
    pub(in crate::net) control_messages: Vec<ControlMessage>,
}

impl MessageHeader {
    /// Creates a new `MessageHeader`.
    pub const fn new(addr: Option<SocketAddr>, control_messages: Vec<ControlMessage>) -> Self {
        Self {
            addr,
            control_messages,
        }
    }

    /// Returns the socket address.
    pub fn addr(&self) -> Option<&SocketAddr> {
        self.addr.as_ref()
    }

    /// Returns the control messages.
    pub fn control_messages(&self) -> &Vec<ControlMessage> {
        &self.control_messages
    }
}

/// Control messages in [`MessageHeader`].
#[derive(Debug)]
pub enum ControlMessage {
    Unix(UnixControlMessage),
}

impl ControlMessage {
    pub fn read_all_from(reader: &mut VmReader) -> Result<Vec<Self>> {
        // FIXME: This method may exhaust kernel memory and cause a panic if the program is
        // malicious and attempts to send too many control messages. To prevent this, we limit the
        // number of control messages, but this limit does not have a Linux equivalent.
        const MAX_NR_MSGS: usize = 32;

        let mut msgs = Vec::new();

        while reader.has_remain() && msgs.len() < MAX_NR_MSGS {
            let header = reader.read_val::<CControlHeader>()?;
            if header.len <= size_of::<CControlHeader>() || header.payload_len() > reader.remain() {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the size of the control message is invalid"
                );
            }

            if let Some(msg) = Self::read_from(&header, reader)? {
                msgs.push(msg);
            }

            let padding_len = header.padding_len().min(reader.remain());
            reader.skip(padding_len);
        }

        if reader.has_remain() {
            warn!("excessive control messages are currently not permitted");
            return_errno_with_message!(
                Errno::E2BIG,
                "excessive control messages are currently not permitted"
            );
        }

        Ok(msgs)
    }

    fn read_from(header: &CControlHeader, reader: &mut VmReader) -> Result<Option<Self>> {
        let Some(level) = header.level() else {
            warn!("unsupported control message level in {:?}", header);
            reader.skip(header.payload_len());
            return Ok(None);
        };

        match level {
            CSocketOptionLevel::SOL_SOCKET => {
                // Linux manual pages say (https://man7.org/linux/man-pages/man7/unix.7.html):
                // "For historical reasons, the ancillary message types listed below are specified
                // with a SOL_SOCKET type even though they are AF_UNIX specific."
                let msg = UnixControlMessage::read_from(header, reader)?;
                Ok(msg.map(Self::Unix))
            }
            _ => {
                warn!("unsupported control message level in {:?}", header);
                reader.skip(header.payload_len());
                Ok(None)
            }
        }
    }

    pub fn write_all_to(msgs: &[Self], writer: &mut VmWriter) -> usize {
        let mut len = 0;

        for msg in msgs.iter() {
            let header = match msg.write_to(writer) {
                Ok(header) => header,
                // This occurs when the buffer is too short or when some page faults cannot be
                // handled. However, at this point, there is no good way to report the errors to
                // user space. According to the Linux implementation, it seems okay to silently
                // ignore errors here.
                Err(_) => {
                    warn!("setting MSG_CTRUNC is not supported");
                    break;
                }
            };

            len += header.total_len();

            let padding_len = header.padding_len().min(writer.avail());
            writer.skip(padding_len);
            len += padding_len;
        }

        len
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<CControlHeader> {
        match self {
            Self::Unix(msg) => msg.write_to(writer),
        }
    }
}

/// `cmsghdr` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/socket.h#L105>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CControlHeader {
    /// Data byte count, including hdr
    len: usize,
    /// Originating protocol
    level: i32,
    /// Protocol-specific type
    type_: i32,
}

/// Alignment of control messages.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/socket.h#L119>.
const CMSG_ALIGN: usize = size_of::<usize>();

impl CControlHeader {
    /// Creates a control message header with the level, type, and payload length.
    pub fn new(level: CSocketOptionLevel, type_: i32, payload_len: usize) -> Self {
        Self {
            len: payload_len + size_of::<Self>(),
            level: level as i32,
            type_,
        }
    }

    /// Computes the payload length from the total length.
    pub fn payload_len_from_total(total_len: usize) -> Result<usize> {
        total_len.checked_sub(size_of::<Self>()).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the control message buffer is too small")
        })
    }

    /// Returns the level of the control message.
    pub fn level(&self) -> Option<CSocketOptionLevel> {
        CSocketOptionLevel::try_from(self.level).ok()
    }

    /// Returns the type of the control message.
    pub fn type_(&self) -> i32 {
        self.type_
    }

    /// Returns the payload length of the control message.
    pub fn payload_len(&self) -> usize {
        self.len - size_of::<Self>()
    }

    /// Returns the length of the control message (payload + header, excluding paddings).
    pub fn total_len(&self) -> usize {
        self.len
    }

    /// Returns the length of the padding bytes for the control message.
    pub(self) fn padding_len(&self) -> usize {
        self.total_len_with_padding() - self.total_len()
    }

    /// Returns the length of the control message (payload + header, including paddings).
    fn total_len_with_padding(&self) -> usize {
        self.len.align_up(CMSG_ALIGN)
    }
}
