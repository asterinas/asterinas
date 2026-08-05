// SPDX-License-Identifier: MPL-2.0

use core::{
    borrow::Borrow,
    ffi::CStr,
    fmt,
    hash::{Hash, Hasher},
    str::Utf8Error,
};

/// An owned C-compatible string with a fixed capacity of `N`.
///
/// Although this is a POD type, it has a type invariant:
/// a nul byte must be present, and all bytes after the first nul byte must also be nul bytes.
/// Users must not arbitrarily mutate the content.
#[repr(C)]
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Pod)]
pub struct FixedCStr<const N: usize>([u8; N]);

impl<const N: usize> FixedCStr<N> {
    /// The maximum byte length, excluding the trailing nul.
    pub const MAX_BYTES: usize = N - 1;

    /// The storage byte length, including the trailing nul.
    pub const MAX_BYTES_WITH_NUL: usize = N;

    /// Creates a `FixedCStr` from bytes, stopping at the first nul byte.
    ///
    /// If there is no nul byte within the first `N` bytes,
    /// the input is truncated to `N` bytes.
    /// The returned field is always nul-terminated,
    /// and all bytes after the first nul byte are zeroed.
    pub fn from_bytes_until_nul(bytes: &[u8]) -> Self {
        const { assert!(N > 0) };

        let mut inner = [0u8; N];
        let len = bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(bytes.len());
        let len = len.min(N - 1);
        inner[..len].copy_from_slice(&bytes[..len]);
        Self(inner)
    }

    pub fn from_str_truncated(str: &str) -> Self {
        Self::from_bytes_until_nul(str.as_bytes())
    }

    pub fn from_cstr_truncated(cstr: &CStr) -> Self {
        Self::from_bytes_until_nul(cstr.to_bytes_with_nul())
    }

    pub fn len(&self) -> usize {
        self.0.iter().position(|&byte| byte == 0).unwrap()
    }

    pub fn is_empty(&self) -> bool {
        self.0[0] == 0
    }

    pub fn as_str(&self) -> Result<&str, Utf8Error> {
        core::str::from_utf8(self.as_bytes())
    }

    pub fn as_cstr(&self) -> &CStr {
        CStr::from_bytes_until_nul(self.0.as_slice()).unwrap()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0[..self.len()]
    }

    pub fn as_bytes_with_nul(&self) -> &[u8] {
        &self.0[..=self.len()]
    }

    /// Returns the full underlying byte array, including trailing nul bytes.
    pub fn as_array(&self) -> &[u8; N] {
        &self.0
    }
}

impl<const N: usize> Default for FixedCStr<N> {
    fn default() -> Self {
        Self([0u8; N])
    }
}

impl<const N: usize> fmt::Debug for FixedCStr<N> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{:?}", self.as_cstr())
    }
}

impl<const N: usize> Borrow<[u8]> for FixedCStr<N> {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<const N: usize> Hash for FixedCStr<N> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

/// An owned C-compatible string with a fixed capacity of `N`.
///
/// Unlike [`FixedCStr`], this type does not require a trailing nul byte.
/// The first nul byte, if any, terminates the string.
/// A string shorter than `N` bytes is padded with nul bytes.
/// A string of exactly `N` bytes has no terminating nul byte.
///
/// Although this is a POD type, it has a type invariant:
/// a nul byte need not be present, but all bytes after the first nul byte must also be nul bytes.
/// Users must not arbitrarily mutate the content.
#[repr(C)]
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Pod)]
pub struct FixedNonTerminatedCStr<const N: usize>([u8; N]);

impl<const N: usize> FixedNonTerminatedCStr<N> {
    pub fn len(&self) -> usize {
        self.0.iter().position(|&byte| byte == 0).unwrap_or(N)
    }

    pub fn is_empty(&self) -> bool {
        self.0.first().is_none_or(|byte| *byte == 0)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0[..self.len()]
    }
}

impl<const N: usize> Default for FixedNonTerminatedCStr<N> {
    fn default() -> Self {
        Self([0u8; N])
    }
}

impl<const N: usize> fmt::Debug for FixedNonTerminatedCStr<N> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match core::str::from_utf8(self.as_bytes()) {
            Ok(string) => write!(formatter, "{}", string),
            Err(_) => write!(formatter, "{:?}", self.as_bytes()),
        }
    }
}
