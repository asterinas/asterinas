// SPDX-License-Identifier: MPL-2.0

use core::{ffi::CStr, mem::offset_of};

use super::family::CSocketAddrFamily;
use crate::{net::socket::unix::UnixSocketAddr, prelude::*};

/// UNIX domain socket address.
///
/// See <https://www.man7.org/linux/man-pages/man7/unix.7.html>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CSocketAddrUnix {
    /// Address family (AF_UNIX).
    sun_family: u16,
    /// Pathname.
    sun_path: [u8; Self::PATH_MAX_LEN],
}

impl CSocketAddrUnix {
    const PATH_MAX_LEN: usize = 108;

    const PATH_OFFSET: usize = offset_of!(Self, sun_path);

    const MIN_LEN: usize = Self::PATH_OFFSET;
    const MAX_LEN: usize = size_of::<Self>();
}

/// Converts a [`UnixSocketAddr`] to bytes representing a [`CSocketAddrUnix`].
///
/// We don't actually create a [`CSocketAddrUnix`]. Instead, we create its byte representation
/// directly for ease of operation.
///
/// # Panics
///
/// This method will panic if the pathname in [`UnixSocketAddr`] is too long to be stored in
/// the `sun_path` field of `CUnixSocketAddr`.
pub(super) fn into_c_bytes_and<R, F>(value: &UnixSocketAddr, f: F) -> R
where
    F: FnOnce(&[u8]) -> R,
{
    // We need to reserve one byte for the null terminator. Because of this, the number of
    // bytes may exceed the size of `CSocketAddrUnix`. This is to match the Linux
    // implementation. See the "BUGS" section at
    // <https://man7.org/linux/man-pages/man7/unix.7.html>.
    let mut bytes: [u8; CSocketAddrUnix::MAX_LEN + 1] = Pod::new_zeroed();

    bytes[..2].copy_from_slice(&(CSocketAddrFamily::AF_UNIX as u16).to_ne_bytes());
    #[allow(clippy::assertions_on_constants)]
    const {
        assert!(CSocketAddrUnix::PATH_OFFSET == 2)
    };

    let sun_path = &mut bytes[CSocketAddrUnix::PATH_OFFSET..];

    let copied = match value {
        UnixSocketAddr::Unnamed => 0,
        UnixSocketAddr::Path(path) => {
            let bytes = path.as_bytes();
            let len = bytes.len();
            sun_path[..len].copy_from_slice(bytes);
            sun_path[len] = 0;
            len + 1
        }
        UnixSocketAddr::Abstract(name) => {
            let len = name.len();
            sun_path[0] = 0;
            sun_path[1..len + 1].copy_from_slice(&name[..]);
            len + 1
        }
    };

    f(&bytes[..CSocketAddrUnix::PATH_OFFSET + copied])
}

/// Converts bytes representing a [`CSocketAddrUnix`] to a [`UnixSocketAddr`].
///
/// We accept the byte representation of a [`CSocketAddrUnix`] directly, instead of
/// [`CSocketAddrUnix`] itself, for ease of operation.
pub(super) fn from_c_bytes(bytes: &[u8]) -> Result<UnixSocketAddr> {
    if bytes.len() < CSocketAddrUnix::MIN_LEN {
        return_errno_with_message!(Errno::EINVAL, "the socket address length is too small");
    }

    if bytes.len() > CSocketAddrUnix::MAX_LEN {
        return_errno_with_message!(Errno::EINVAL, "the socket address length is too small");
    }

    let sun_path = &bytes[CSocketAddrUnix::PATH_OFFSET..];

    if sun_path.is_empty() {
        return Ok(UnixSocketAddr::Unnamed);
    }

    if sun_path[0] == 0 {
        return Ok(UnixSocketAddr::Abstract(Arc::from(&sun_path[1..])));
    }

    // Again, Linux always appends a null terminator to the pathname if none is supplied. So we
    // need to deal with the case where `CStr::from_bytes_until_nul` fails.
    if let Ok(c_str) = CStr::from_bytes_until_nul(sun_path) {
        Ok(UnixSocketAddr::Path(Arc::from(c_str.to_string_lossy())))
    } else {
        Ok(UnixSocketAddr::Path(Arc::from(String::from_utf8_lossy(
            sun_path,
        ))))
    }
}
