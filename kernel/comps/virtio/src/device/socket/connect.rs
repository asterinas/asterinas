use log::debug;

use super::{header::{VsockAddr, VirtioVsockHdr, VirtioVsockOp}, error::SocketError};


#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VsockBufferStatus {
    pub buffer_allocation: u32,
    pub forward_count: u32,
}

/// The reason why a vsock connection was closed.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DisconnectReason {
    /// The peer has either closed the connection in response to our shutdown request, or forcibly
    /// closed it of its own accord.
    Reset,
    /// The peer asked to shut down the connection.
    Shutdown,
}

/// Details of the type of an event received from a VirtIO socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VsockEventType {
    /// The peer requests to establish a connection with us.
    ConnectionRequest,
    /// The connection was successfully established.
    Connected,
    /// The connection was closed.
    Disconnected {
        /// The reason for the disconnection.
        reason: DisconnectReason,
    },
    /// Data was received on the connection.
    Received {
        /// The length of the data in bytes.
        length: usize,
    },
    /// The peer requests us to send a credit update.
    CreditRequest,
    /// The peer just sent us a credit update with nothing else.
    CreditUpdate,
}

/// An event received from a VirtIO socket device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VsockEvent {
    /// The source of the event, i.e. the peer who sent it.
    pub source: VsockAddr,
    /// The destination of the event, i.e. the CID and port on our side.
    pub destination: VsockAddr,
    /// The peer's buffer status for the connection.
    pub buffer_status: VsockBufferStatus,
    /// The type of event.
    pub event_type: VsockEventType,
}

impl VsockEvent {
    /// Returns whether the event matches the given connection.
    pub fn matches_connection(&self, connection_info: &ConnectionInfo, guest_cid: u64) -> bool {
        self.source == connection_info.dst
            && self.destination.cid == guest_cid
            && self.destination.port == connection_info.src_port
    }

    pub fn from_header(header: &VirtioVsockHdr) -> Result<Self,SocketError> {
        let op = header.op()?;
        let buffer_status = VsockBufferStatus {
            buffer_allocation: header.buf_alloc,
            forward_count: header.fwd_cnt,
        };
        let source = header.source();
        let destination = header.destination();

        let event_type = match op {
            VirtioVsockOp::Request => {
                header.check_data_is_empty()?;
                VsockEventType::ConnectionRequest
            }
            VirtioVsockOp::Response => {
                header.check_data_is_empty()?;
                VsockEventType::Connected
            }
            VirtioVsockOp::CreditUpdate => {
                header.check_data_is_empty()?;
                VsockEventType::CreditUpdate
            }
            VirtioVsockOp::Rst | VirtioVsockOp::Shutdown => {
                header.check_data_is_empty()?;
                debug!("Disconnected from the peer");
                let reason = if op == VirtioVsockOp::Rst {
                    DisconnectReason::Reset
                } else {
                    DisconnectReason::Shutdown
                };
                VsockEventType::Disconnected { reason }
            }
            VirtioVsockOp::Rw => VsockEventType::Received {
                length: header.len() as usize,
            },
            VirtioVsockOp::CreditRequest => {
                header.check_data_is_empty()?;
                VsockEventType::CreditRequest
            }
            VirtioVsockOp::Invalid => return Err(SocketError::InvalidOperation),
        };

        Ok(VsockEvent {
            source,
            destination,
            buffer_status,
            event_type,
        })
    }
}


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub dst: VsockAddr,
    pub src_port: u32,
    /// The last `buf_alloc` value the peer sent to us, indicating how much receive buffer space in
    /// bytes it has allocated for packet bodies.
    peer_buf_alloc: u32,
    /// The last `fwd_cnt` value the peer sent to us, indicating how many bytes of packet bodies it
    /// has finished processing.
    peer_fwd_cnt: u32,
    /// The number of bytes of packet bodies which we have sent to the peer.
    pub tx_cnt: u32,
    /// The number of bytes of buffer space we have allocated to receive packet bodies from the
    /// peer.
    pub buf_alloc: u32,
    /// The number of bytes of packet bodies which we have received from the peer and handled.
    pub fwd_cnt: u32,
    /// Whether we have recently requested credit from the peer.
    ///
    /// This is set to true when we send a `VIRTIO_VSOCK_OP_CREDIT_REQUEST`, and false when we
    /// receive a `VIRTIO_VSOCK_OP_CREDIT_UPDATE`.
    pub has_pending_credit_request: bool,
}

impl ConnectionInfo {
    pub fn new(destination: VsockAddr, src_port: u32) -> Self {
        Self {
            dst: destination,
            src_port,
            ..Default::default()
        }
    }

    /// Updates this connection info with the peer buffer allocation and forwarded count from the
    /// given event.
    pub fn update_for_event(&mut self, event: &VsockEvent) {
        self.peer_buf_alloc = event.buffer_status.buffer_allocation;
        self.peer_fwd_cnt = event.buffer_status.forward_count;

        if let VsockEventType::CreditUpdate = event.event_type {
            self.has_pending_credit_request = false;
        }
    }

    /// Increases the forwarded count recorded for this connection by the given number of bytes.
    ///
    /// This should be called once received data has been passed to the client, so there is buffer
    /// space available for more.
    pub fn done_forwarding(&mut self, length: usize) {
        self.fwd_cnt += length as u32;
    }

    /// Returns the number of bytes of RX buffer space the peer has available to receive packet body
    /// data from us.
    pub fn peer_free(&self) -> u32 {
        self.peer_buf_alloc - (self.tx_cnt - self.peer_fwd_cnt)
    }

    pub fn new_header(&self, src_cid: u64) -> VirtioVsockHdr {
        VirtioVsockHdr {
            src_cid,
            dst_cid: self.dst.cid,
            src_port: self.src_port,
            dst_port: self.dst.port,
            buf_alloc: self.buf_alloc,
            fwd_cnt: self.fwd_cnt,
            ..Default::default()
        }
    }
}