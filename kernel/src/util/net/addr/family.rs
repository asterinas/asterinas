// SPDX-License-Identifier: MPL-2.0

use core::cmp::min;

use ostd::task::Task;

use super::{ip::CSocketAddrInet, unix, vsock::CSocketAddrVm};
use crate::{current_userspace, net::socket::SocketAddr, prelude::*};

/// Address family.
///
/// See <https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h>.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
pub enum CSocketAddrFamily {
    AF_UNSPEC = 0,
    /// Unix domain sockets
    AF_UNIX = 1,
    // POSIX name for AF_UNIX
    // AF_LOCAL = 1,
    /// Internet IP Protocol
    AF_INET = 2,
    /// Amateur Radio AX.25
    AF_AX25 = 3,
    /// Novell IPX
    AF_IPX = 4,
    /// AppleTalk DDP
    AF_APPLETALK = 5,
    /// Amateur Radio NET/ROM
    AF_NETROM = 6,
    /// Multiprotocol bridge
    AF_BRIDGE = 7,
    /// ATM PVCs
    AF_ATMPVC = 8,
    /// Reserved for X.25 project
    AF_X25 = 9,
    /// IP version 6,
    AF_INET6 = 10,
    /// Amateur Radio X.25 PLP
    AF_ROSE = 11,
    /// Reserved for DECnet project
    AF_DECnet = 12,
    /// Reserved for 802.2LLC project
    AF_NETBEUI = 13,
    /// Security callback pseudo AF
    AF_SECURITY = 14,
    /// PF_KEY key management API
    AF_KEY = 15,
    AF_NETLINK = 16,
    // Alias to emulate 4.4BSD
    // AF_ROUTE = AF_NETLINK
    /// Packet family
    AF_PACKET = 17,
    /// Ash
    AF_ASH = 18,
    /// Acorn Econet
    AF_ECONET = 19,
    /// ATM SVCs
    AF_ATMSVC = 20,
    /// RDS sockets
    AF_RDS = 21,
    /// Linux SNA Project (nutters!)
    AF_SNA = 22,
    /// IRDA sockets
    AF_IRDA = 23,
    /// PPPoX sockets
    AF_PPPOX = 24,
    /// Wanpipe API Sockets
    AF_WANPIPE = 25,
    /// Linux LLC
    AF_LLC = 26,
    /// Native InfiniBand address
    AF_IB = 27,
    /// MPLS
    AF_MPLS = 28,
    /// Controller Area Network
    AF_CAN = 29,
    /// TIPC sockets
    AF_TIPC = 30,
    /// Bluetooth sockets
    AF_BLUETOOTH = 31,
    /// IUCV sockets
    AF_IUCV = 32,
    /// RxRPC sockets
    AF_RXRPC = 33,
    /// mISDN sockets
    AF_ISDN = 34,
    /// Phonet sockets
    AF_PHONET = 35,
    /// IEEE802154 sockets
    AF_IEEE802154 = 36,
    /// CAIF sockets
    AF_CAIF = 37,
    /// Algorithm sockets
    AF_ALG = 38,
    /// NFC sockets
    AF_NFC = 39,
    /// vSockets
    AF_VSOCK = 40,
    /// Kernel Connection Multiplexor
    AF_KCM = 41,
    /// Qualcomm IPC Router
    AF_QIPCRTR = 42,
    /// smc sockets: reserve number for
    /// PF_SMC protocol family that
    /// reuses AF_INET address family
    AF_SMC = 43,
    /// XDP sockets
    AF_XDP = 44,
    /// Management component transport protocol
    AF_MCTP = 45,
}

const ADDR_MAX_LEN: usize = 128;

/// Storage that can contain _any_ socket addresses.
///
/// The size and layout of this structure is specified by RFC 3493. For details, see
/// <https://datatracker.ietf.org/doc/html/rfc3493#section-3.10>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct Storage {
    sa_family: u16,
    bytes: [u8; ADDR_MAX_LEN - 2],
    _align: [u64; 0],
}

/// Reads a socket address from userspace.
///
/// This method returns `Err(EINVAL)` for invalid socket address lengths and `Err(EAFNOSUPPORT)`
/// for unsupported address families.
///
/// These error codes may be different from the Linux ones, but the difference is tricky and hard
/// to fix. The main reason for this is that in Linux it's up to each protocol to decide how to
/// intercept the bytes that represent socket addresses, but here this method is designed to parse
/// socket addresses before diving deep into protocol-specific code.
pub fn read_socket_addr_from_user(addr: Vaddr, addr_len: usize) -> Result<SocketAddr> {
    if addr_len > ADDR_MAX_LEN {
        return_errno_with_message!(Errno::EINVAL, "the socket address length is too large");
    }

    if addr_len < 2 {
        return_errno_with_message!(Errno::EINVAL, "the socket address length is too small");
    }

    let mut storage = Storage::new_zeroed();
    current_userspace!().read_bytes(
        addr,
        &mut VmWriter::from(&mut storage.as_bytes_mut()[..addr_len]),
    )?;

    let result = match CSocketAddrFamily::try_from(storage.sa_family as i32) {
        Ok(CSocketAddrFamily::AF_INET) => {
            if addr_len < size_of::<CSocketAddrInet>() {
                return_errno_with_message!(Errno::EINVAL, "the socket address length is too small");
            }
            let (addr, port) = CSocketAddrInet::from_bytes(storage.as_bytes()).into();
            SocketAddr::IPv4(addr, port)
        }
        Ok(CSocketAddrFamily::AF_UNIX) => {
            let addr = unix::from_c_bytes(&storage.as_bytes()[..addr_len])?;
            SocketAddr::Unix(addr)
        }
        Ok(CSocketAddrFamily::AF_VSOCK) => {
            if addr_len < size_of::<CSocketAddrVm>() {
                return_errno_with_message!(Errno::EINVAL, "the socket address length is too small");
            }
            let addr = CSocketAddrVm::from_bytes(storage.as_bytes());
            SocketAddr::Vsock(addr.into())
        }
        _ => {
            return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the specified address family is not supported"
            )
        }
    };

    Ok(result)
}

/// Writes a socket address and its length to userspace.
///
/// Similar to [`write_socket_addr_with_max_len`], the socket address may be truncated if the
/// buffer is not long enough. Even if truncation occurs, the actual length of the socket address
/// is written to userspace. See <https://man7.org/linux/man-pages/man3/recvmsg.3p.html> for
/// details on this behavior.
///
/// # Panics
///
/// This method will panic if the socket address cannot be validly mapped to the corresponding
/// Linux C structures. Currently, the only possible example is that the pathname in the UNIX
/// domain socket address is too long.
///
/// It is guaranteed that all socket addresses returned by [`read_socket_addr_from_user`] have
/// valid representations for the corresponding C structures, so passing them to this method will
/// not cause panic.
pub fn write_socket_addr_to_user(
    socket_addr: &SocketAddr,
    dest: Vaddr,
    max_len_ptr: Vaddr,
) -> Result<()> {
    let current_task = Task::current().unwrap();
    let user_space = CurrentUserSpace::new(&current_task);

    let max_len = user_space.read_val::<i32>(max_len_ptr)?;

    let actual_len = write_socket_addr_with_max_len(socket_addr, dest, max_len)?;

    user_space.write_val(max_len_ptr, &actual_len)
}

/// Writes a socket address to the user space.
///
/// If the specified maximum length for the socket address is not enough, the socket address is
/// truncated to the specified maximum length. This method returns the _actual_ length of the
/// socket address, regardless of whether the truncation occurs or not.
///
/// # Panics
///
/// This method will panic if the socket address cannot be validly mapped to the corresponding
/// Linux C structures. Currently, the only possible example is that the pathname in the UNIX
/// domain socket address is too long.
///
/// It is guaranteed that all socket addresses returned by [`read_socket_addr_from_user`] have
/// valid representations for the corresponding C structures, so passing them to this method will
/// not cause panic.
pub fn write_socket_addr_with_max_len(
    socket_addr: &SocketAddr,
    dest: Vaddr,
    max_len: i32,
) -> Result<i32> {
    if max_len < 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "the socket address length cannot be negative"
        );
    }

    let current_task = Task::current().unwrap();
    let user_space = CurrentUserSpace::new(&current_task);

    let actual_len = match socket_addr {
        SocketAddr::IPv4(addr, port) => {
            let socket_addr = CSocketAddrInet::from((*addr, *port));
            let actual_len = size_of::<CSocketAddrInet>();
            let written_len = min(actual_len, max_len as _);
            user_space.write_bytes(
                dest,
                &mut VmReader::from(&socket_addr.as_bytes()[..written_len]),
            )?;
            actual_len
        }
        SocketAddr::Unix(addr) => unix::into_c_bytes_and(addr, |bytes| {
            let written_len = min(bytes.len(), max_len as _);
            user_space.write_bytes(dest, &mut VmReader::from(&bytes[..written_len]))?;
            Ok::<usize, Error>(bytes.len())
        })?,
        SocketAddr::Vsock(addr) => {
            let socket_addr = CSocketAddrVm::from(*addr);
            let actual_len = size_of::<CSocketAddrVm>();
            let written_len = min(actual_len, max_len as _);
            user_space.write_bytes(
                dest,
                &mut VmReader::from(&socket_addr.as_bytes()[..written_len]),
            )?;
            actual_len
        }
    };

    Ok(actual_len as i32)
}
