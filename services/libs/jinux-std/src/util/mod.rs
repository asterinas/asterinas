use crate::{
    net::socket::SocketAddr,
    prelude::*,
    util::net::{InAddr, SaFamily, SockAddr, SockAddrIn, SockAddrIn6, SockAddrUn},
};
use jinux_frame::vm::VmIo;
pub mod net;

/// copy bytes from user space of current process. The bytes len is the len of dest.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut [u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_bytes(src, dest)?)
}

/// copy val (Plain of Data type) from user space of current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> Result<T> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_val(src)?)
}

/// write bytes from user space of current process. The bytes len is the len of src.
pub fn write_bytes_to_user(dest: Vaddr, src: &[u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_bytes(dest, src)?)
}

/// write val (Plain of Data type) to user space of current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_val(dest, val)?)
}

/// read a cstring from user, the length of cstring should not exceed max_len(include null byte)
pub fn read_cstring_from_user(addr: Vaddr, max_len: usize) -> Result<CString> {
    let mut buffer = vec![0u8; max_len];
    read_bytes_from_user(addr, &mut buffer)?;
    Ok(CString::from(CStr::from_bytes_until_nul(&buffer)?))
}

pub fn read_socket_addr_from_user(addr: Vaddr, addr_len: usize) -> Result<SocketAddr> {
    debug_assert!(addr_len >= core::mem::size_of::<SockAddr>());
    let sockaddr: SockAddr = read_val_from_user(addr)?;
    let socket_addr = match sockaddr.sa_family()? {
        SaFamily::AF_UNSPEC => {
            return_errno_with_message!(Errno::EINVAL, "the socket addr family is unspecified")
        }
        SaFamily::AF_UNIX => {
            debug_assert!(addr_len >= core::mem::size_of::<SockAddrUn>());
            let sock_addr_un: SockAddrUn = read_val_from_user(addr)?;
            todo!()
        }
        SaFamily::AF_INET => {
            debug_assert!(addr_len >= core::mem::size_of::<SockAddrIn>());
            let sock_addr_in: SockAddrIn = read_val_from_user(addr)?;
            SocketAddr::from(sock_addr_in)
        }
        SaFamily::AF_INET6 => {
            debug_assert!(addr_len >= core::mem::size_of::<SockAddrIn6>());
            let sock_addr_in6: SockAddrIn6 = read_val_from_user(addr)?;
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
    max_len: usize,
) -> Result<usize> {
    match socket_addr {
        SocketAddr::Unix => todo!(),
        SocketAddr::IPv4(addr, port) => {
            let in_addr = InAddr::from(*addr);
            let sock_addr_in = SockAddrIn::new(*port, in_addr);
            let write_size = core::mem::size_of::<SockAddrIn>();
            debug_assert!(max_len >= write_size);
            write_val_to_user(dest, &sock_addr_in)?;
            Ok(write_size)
        }
        SocketAddr::IPv6 => todo!(),
    }
}
