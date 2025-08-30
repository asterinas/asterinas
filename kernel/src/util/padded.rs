// SPDX-License-Identifier: MPL-2.0

/// Creates a fixed-size byte array out of a byte slice.
///
/// # Example
///
/// ```
/// let fixed_size_text: [u8; 128] = padded(b"Hello World");
/// ```
///
/// Without this `padded` utility function,
/// one would have to write a more lengthy but less efficient version.
///
/// ```
/// let fixed_size_text: [u8; 128] = {
///    const HELLO: &[u8] = b"Hello World";
///    let mut buf = [0u8; 128];
///    buf[..HELLO.len()].copy_from_slice(HELLO);
///    buf
/// };
/// ```
pub const fn padded<const N: usize>(s: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut i = 0;
    while i < s.len() && i < N {
        out[i] = s[i];
        i += 1;
    }
    out
}
