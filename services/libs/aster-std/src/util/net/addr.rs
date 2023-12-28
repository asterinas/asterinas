use crate::net::iface::Ipv4Address;
use crate::net::socket::unix::UnixSocketAddr;
use crate::net::socket::SocketAddr;
use crate::prelude::*;
use crate::util::{read_bytes_from_user, read_val_from_user, write_val_to_user};

pub fn read_socket_addr_from_user(addr: Vaddr, addr_len: usize) -> Result<SocketAddr> {
    debug_assert!(addr_len >= core::mem::size_of::<CSocketAddr>());
    let sockaddr: CSocketAddr = read_val_from_user(addr)?;
    let socket_addr = match sockaddr.sa_family()? {
        CSocketAddrFamily::AF_UNSPEC => {
            return_errno_with_message!(Errno::EINVAL, "the socket addr family is unspecified")
        }
        CSocketAddrFamily::AF_UNIX => {
            debug_assert!(addr_len >= core::mem::size_of::<CSocketAddr>());
            let sa_family: u16 = read_val_from_user(addr)?;
            debug_assert!(sa_family == CSocketAddrFamily::AF_UNIX as u16);

            let bytes = {
                let bytes_len = addr_len - core::mem::size_of::<u16>();
                let mut bytes = vec![0u8; bytes_len];
                read_bytes_from_user(addr + core::mem::size_of::<u16>(), &mut bytes)?;
                bytes
            };

            let unix_socket_addr = if bytes.starts_with(&[0]) {
                // Abstract unix socket addr
                let cstr = CStr::from_bytes_until_nul(&bytes[1..])?;
                let abstract_path = cstr.to_string_lossy().to_string();
                UnixSocketAddr::Abstract(abstract_path)
            } else {
                // Normal unix sockket addr
                let cstr = CStr::from_bytes_until_nul(&bytes)?;
                let path = cstr.to_string_lossy().to_string();
                UnixSocketAddr::Path(path)
            };

            SocketAddr::Unix(unix_socket_addr)
        }
        CSocketAddrFamily::AF_INET => {
            debug_assert!(addr_len >= core::mem::size_of::<CSocketAddrInet>());
            let sock_addr_in: CSocketAddrInet = read_val_from_user(addr)?;
            SocketAddr::from(sock_addr_in)
        }
        CSocketAddrFamily::AF_INET6 => {
            debug_assert!(addr_len >= core::mem::size_of::<CSocketAddrInet6>());
            let sock_addr_in6: CSocketAddrInet6 = read_val_from_user(addr)?;
            todo!()
        }
        _ => {
            return_errno_with_message!(Errno::EAFNOSUPPORT, "cannot support address for the family")
        }
    };
    Ok(socket_addr)
}

pub fn write_socket_addr_to_user(
    socket_addr: &SocketAddr,
    dest: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<()> {
    debug_assert!(addrlen_ptr != 0);
    if addrlen_ptr == 0 {
        return_errno_with_message!(Errno::EINVAL, "must provide the addrlen ptr");
    }
    let max_len = read_val_from_user::<i32>(addrlen_ptr)? as usize;
    let write_size = match socket_addr {
        SocketAddr::Unix(path) => {
            let sock_addr_unix = CSocketAddrUnix::try_from(path)?;
            let write_size = core::mem::size_of::<CSocketAddrUnix>();
            debug_assert!(max_len >= write_size);
            write_val_to_user(dest, &sock_addr_unix)?;
            write_size as i32
        }
        SocketAddr::IPv4(addr, port) => {
            let in_addr = CInetAddr::from(*addr);
            let sock_addr_in = CSocketAddrInet::new(*port, in_addr);
            let write_size = core::mem::size_of::<CSocketAddrInet>();
            debug_assert!(max_len >= write_size);
            write_val_to_user(dest, &sock_addr_in)?;
            write_size as i32
        }
        SocketAddr::IPv6 => todo!(),
    };
    if addrlen_ptr != 0 {
        write_val_to_user(addrlen_ptr, &write_size)?;
    }
    Ok(())
}

/// PlaceHolder
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CSocketAddr {
    sa_family: u16, // SaFamily
    sa_data: [u8; 14],
}

impl CSocketAddr {
    pub fn sa_family(&self) -> Result<CSocketAddrFamily> {
        Ok(CSocketAddrFamily::try_from(self.sa_family as i32)?)
    }
}

const SOCKET_ADDR_UNIX_LEN: usize = 108;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CSocketAddrUnix {
    sun_family: u16, // Always SaFamily::AF_UNIX
    sun_path: [u8; SOCKET_ADDR_UNIX_LEN],
}

/// IPv4 4-byte address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CInetAddr {
    s_addr: [u8; 4],
}

impl CInetAddr {
    pub fn as_bytes(&self) -> &[u8] {
        &self.s_addr
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        debug_assert!(bytes.len() == 4);
        let mut s_addr = [0u8; 4];
        s_addr.copy_from_slice(bytes);
        Self { s_addr }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CPortNum {
    port: [u8; 2],
}

impl CPortNum {
    pub fn as_u16(&self) -> u16 {
        u16::from_be_bytes(self.port)
    }

    pub fn from_u16(value: u16) -> Self {
        let bytes = value.to_be_bytes();
        Self { port: bytes }
    }
}

/// IPv4 socket address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CSocketAddrInet {
    /// always SaFamily::AF_INET
    sin_family: u16,
    /// Port number
    sin_port_t: CPortNum,
    /// IPv4 address
    sin_addr: CInetAddr,
    /// Pad to size of 'SockAddr' structure (16 bytes)
    _pad: [u8; 8],
}

impl CSocketAddrInet {
    pub fn new(port: u16, addr: CInetAddr) -> Self {
        let port = CPortNum::from_u16(port);
        Self {
            sin_family: CSocketAddrFamily::AF_INET as _,
            sin_port_t: port,
            sin_addr: addr,
            _pad: [0u8; 8],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CInet6Addr {
    s6_addr: [u8; 16],
}

impl CInet6Addr {
    pub fn as_bytes(&self) -> &[u8] {
        &self.s6_addr
    }
}

/// IPv6 socket address
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CSocketAddrInet6 {
    /// always SaFamily::AF_INET6
    sin6_family: u16,
    /// Port number
    sin6_port: CPortNum,
    /// IPv6 flow information
    sin6_flowinfo: u32,
    /// IPv6 address
    sin6_addr: CInet6Addr,
    // Scope ID
    sin6_scope_id: u32,
}

/// Address family. The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum CSocketAddrFamily {
    AF_UNSPEC = 0,
    AF_UNIX = 1, /* Unix domain sockets 		*/
    //AF_LOCAL	1	/* POSIX name for AF_UNIX	*/
    AF_INET = 2,      /* Internet IP Protocol 	*/
    AF_AX25 = 3,      /* Amateur Radio AX.25 		*/
    AF_IPX = 4,       /* Novell IPX 			*/
    AF_APPLETALK = 5, /* AppleTalk DDP 		*/
    AF_NETROM = 6,    /* Amateur Radio NET/ROM 	*/
    AF_BRIDGE = 7,    /* Multiprotocol bridge 	*/
    AF_ATMPVC = 8,    /* ATM PVCs			*/
    AF_X25 = 9,       /* Reserved for X.25 project 	*/
    AF_INET6 = 10,    /* IP version 6			*/
    AF_ROSE = 11,     /* Amateur Radio X.25 PLP	*/
    AF_DECnet = 12,   /* Reserved for DECnet project	*/
    AF_NETBEUI = 13,  /* Reserved for 802.2LLC project*/
    AF_SECURITY = 14, /* Security callback pseudo AF */
    AF_KEY = 15,      /* PF_KEY key management API */
    AF_NETLINK = 16,
    //AF_ROUTE	= AF_NETLINK /* Alias to emulate 4.4BSD */
    AF_PACKET = 17,     /* Packet family		*/
    AF_ASH = 18,        /* Ash				*/
    AF_ECONET = 19,     /* Acorn Econet			*/
    AF_ATMSVC = 20,     /* ATM SVCs			*/
    AF_RDS = 21,        /* RDS sockets 			*/
    AF_SNA = 22,        /* Linux SNA Project (nutters!) */
    AF_IRDA = 23,       /* IRDA sockets			*/
    AF_PPPOX = 24,      /* PPPoX sockets		*/
    AF_WANPIPE = 25,    /* Wanpipe API Sockets */
    AF_LLC = 26,        /* Linux LLC			*/
    AF_IB = 27,         /* Native InfiniBand address	*/
    AF_MPLS = 28,       /* MPLS */
    AF_CAN = 29,        /* Controller Area Network      */
    AF_TIPC = 30,       /* TIPC sockets			*/
    AF_BLUETOOTH = 31,  /* Bluetooth sockets 		*/
    AF_IUCV = 32,       /* IUCV sockets			*/
    AF_RXRPC = 33,      /* RxRPC sockets 		*/
    AF_ISDN = 34,       /* mISDN sockets 		*/
    AF_PHONET = 35,     /* Phonet sockets		*/
    AF_IEEE802154 = 36, /* IEEE802154 sockets		*/
    AF_CAIF = 37,       /* CAIF sockets			*/
    AF_ALG = 38,        /* Algorithm sockets		*/
    AF_NFC = 39,        /* NFC sockets			*/
    AF_VSOCK = 40,      /* vSockets			*/
    AF_KCM = 41,        /* Kernel Connection Multiplexor*/
    AF_QIPCRTR = 42,    /* Qualcomm IPC Router          */
    AF_SMC = 43,        /* smc sockets: reserve number for
                         * PF_SMC protocol family that
                         * reuses AF_INET address family
                         */
    AF_XDP = 44, /* XDP sockets			*/
    AF_MCTP = 45, /* Management component
                  * transport protocol
                  */
    AF_MAX = 46, /* For now.. */
}

impl From<CInetAddr> for Ipv4Address {
    fn from(value: CInetAddr) -> Self {
        let addr = value.as_bytes();
        Ipv4Address::from_bytes(addr)
    }
}

impl From<Ipv4Address> for CInetAddr {
    fn from(value: Ipv4Address) -> Self {
        let bytes = value.as_bytes();
        CInetAddr::from_bytes(bytes)
    }
}

impl From<CSocketAddrInet> for SocketAddr {
    fn from(value: CSocketAddrInet) -> Self {
        let port = value.sin_port_t.as_u16();
        let addr = Ipv4Address::from(value.sin_addr);
        SocketAddr::IPv4(addr, port)
    }
}

impl TryFrom<&UnixSocketAddr> for CSocketAddrUnix {
    type Error = Error;

    fn try_from(value: &UnixSocketAddr) -> Result<Self> {
        let mut sun_path = [0u8; SOCKET_ADDR_UNIX_LEN];
        match value {
            UnixSocketAddr::Path(path) => {
                let bytes = path.as_bytes();
                let copy_len = bytes.len().min(SOCKET_ADDR_UNIX_LEN - 1);
                sun_path[..copy_len].copy_from_slice(&bytes[..copy_len]);
                Ok(CSocketAddrUnix {
                    sun_family: CSocketAddrFamily::AF_UNIX as u16,
                    sun_path,
                })
            }
            UnixSocketAddr::Abstract(_) => todo!(),
        }
    }
}
