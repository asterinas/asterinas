// SPDX-License-Identifier: MPL-2.0

//! This module implements hash algorithms developed by Bob Jenkins.
//!
//! For further information, visit: www.burtleburtle.net/bob/hash/doobs.html
//!
//! The hash functions in this module facilitate the production of 32-bit values for hash table lookups.
//! Although the [Linux kernel's jhash](https://github.com/torvalds/linux/blob/master/include/linux/jhash.h)
//! slightly differs from the original version, this module reimplements the Linux kernel's version.
//!

#![no_std]
#![deny(unsafe_code)]

/// A randomly chosen initial value
const JHASH_INITVAL: u32 = 0xdeadbeef;

/// Hashes an arbitrary u8 slices.
///
/// # Example
///
/// If you are hashing n slices, do it like this:
/// ```rust
/// use jhash::jhash_slice;
///
/// fn hash_slices(slices: &[&[u8]]) -> u32 {
///     let mut hash: u32 = 0;
///     for slice in slices {
///         hash = jhash_slice(slice, hash);
///     }
///     hash
/// }
/// ```
pub const fn jhash_slice(slice: &[u8], initval: u32) -> u32 {
    let mut length = slice.len() as u32;
    let mut index: usize = 0;

    // Init the internal state
    let mut a: u32 = JHASH_INITVAL.wrapping_add(length).wrapping_add(initval);
    let mut b: u32 = a;
    let mut c: u32 = a;

    // Handle the most bytes except last 12 bytes
    while length > 12 {
        // FIXME: The Linux version uses `u32::from_ne_bytes`.
        // Here, we use `from_le_bytes` to ensure consistency
        // in results across multiple runs on all machines.
        a = a.wrapping_add(u32::from_le_bytes([
            slice[index],
            slice[index + 1],
            slice[index + 2],
            slice[index + 3],
        ]));
        b = b.wrapping_add(u32::from_le_bytes([
            slice[index + 4],
            slice[index + 5],
            slice[index + 6],
            slice[index + 7],
        ]));
        c = c.wrapping_add(u32::from_le_bytes([
            slice[index + 8],
            slice[index + 9],
            slice[index + 10],
            slice[index + 11],
        ]));
        (a, b, c) = jhash_mix(a, b, c);

        index += 12;
        length -= 12;
    }

    // Handle the last 12 bytes
    if length == 12 {
        c = c.wrapping_add((slice[index + 11] as u32) << 24);
    }

    if length >= 11 {
        c = c.wrapping_add((slice[index + 10] as u32) << 16);
    }

    if length >= 10 {
        c = c.wrapping_add((slice[index + 9] as u32) << 8);
    }

    if length >= 9 {
        c = c.wrapping_add(slice[index + 8] as u32);
    }

    if length >= 8 {
        b = b.wrapping_add((slice[index + 7] as u32) << 24);
    }

    if length >= 7 {
        b = b.wrapping_add((slice[index + 6] as u32) << 16);
    }

    if length >= 6 {
        b = b.wrapping_add((slice[index + 5] as u32) << 8);
    }

    if length >= 5 {
        b = b.wrapping_add(slice[index + 4] as u32);
    }

    if length >= 4 {
        a = a.wrapping_add((slice[index + 3] as u32) << 24);
    }

    if length >= 3 {
        a = a.wrapping_add((slice[index + 2] as u32) << 16);
    }

    if length >= 2 {
        a = a.wrapping_add((slice[index + 1] as u32) << 8);
    }

    if length >= 1 {
        a = a.wrapping_add(slice[index] as u32);
        return jhash_final(a, b, c);
    }

    c
}

/// Hashes exactly three u32 values
pub const fn jhash_3vals(a: u32, b: u32, c: u32, initval: u32) -> u32 {
    jhash_3vals_inner(
        a,
        b,
        c,
        initval.wrapping_add(JHASH_INITVAL).wrapping_add(3 << 2),
    )
}

/// Hashes exactly two u32 values
pub const fn jhash_2vals(a: u32, b: u32, initval: u32) -> u32 {
    jhash_3vals_inner(
        a,
        b,
        0,
        initval.wrapping_add(JHASH_INITVAL).wrapping_add(2 << 2),
    )
}

/// Hashes exactly one u32 value
pub const fn jhash_1vals(a: u32, initval: u32) -> u32 {
    jhash_3vals_inner(
        a,
        0,
        0,
        initval.wrapping_add(JHASH_INITVAL).wrapping_add(1 << 2),
    )
}

/// Hashes an array of u32 values.
pub const fn jhash_u32_array(array: &[u32], initval: u32) -> u32 {
    let mut length = array.len() as u32;
    let mut index = 0;

    // Initialize values a, b, and c
    let mut a: u32 = JHASH_INITVAL
        .wrapping_add(length << 2)
        .wrapping_add(initval);
    let mut b: u32 = a;
    let mut c: u32 = a;

    // Process most values except the last three
    while length > 3 {
        a = a.wrapping_add(array[index]);
        b = b.wrapping_add(array[index + 1]);
        c = c.wrapping_add(array[index + 2]);
        (a, b, c) = jhash_mix(a, b, c);

        length -= 3;
        index += 3;
    }

    if length == 3 {
        c = c.wrapping_add(array[index + 2]);
    }

    if length >= 2 {
        b = b.wrapping_add(array[index + 1]);
    }

    if length >= 1 {
        a = a.wrapping_add(array[index]);
        return jhash_final(a, b, c);
    }

    c
}

/// An internal function that handles hashing for 3 u32 values
const fn jhash_3vals_inner(mut a: u32, mut b: u32, mut c: u32, initval: u32) -> u32 {
    a = a.wrapping_add(initval);
    b = b.wrapping_add(initval);
    c = c.wrapping_add(initval);

    jhash_final(a, b, c)
}

/// Finalizes the mix of three 32-bit values into a single u32 value
const fn jhash_final(mut a: u32, mut b: u32, mut c: u32) -> u32 {
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(14));

    a ^= c;
    a = a.wrapping_sub(c.rotate_left(11));

    b ^= a;
    b = b.wrapping_sub(a.rotate_left(25));

    c ^= b;
    c = c.wrapping_sub(b.rotate_left(16));

    a ^= c;
    a = a.wrapping_sub(c.rotate_left(4));

    b ^= a;
    b = b.wrapping_sub(a.rotate_left(14));

    c ^= b;
    c.wrapping_sub(b.rotate_left(24))
}

/// Mixes three 32-bit values in a reversible manner
const fn jhash_mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
    a = a.wrapping_sub(c);
    a ^= c.rotate_left(4);
    c = c.wrapping_add(b);

    b = b.wrapping_sub(a);
    b ^= a.rotate_left(6);
    a = a.wrapping_add(c);

    c = c.wrapping_sub(b);
    c ^= b.rotate_left(8);
    b = b.wrapping_add(a);

    a = a.wrapping_sub(c);
    a ^= c.rotate_left(16);
    c = c.wrapping_add(b);

    b = b.wrapping_sub(a);
    b ^= a.rotate_left(19);
    a = a.wrapping_add(c);

    c = c.wrapping_sub(b);
    c ^= b.rotate_left(4);
    b = b.wrapping_add(a);

    (a, b, c)
}

#[cfg(test)]
mod test {
    use super::*;

    const JHASH_INITVAL: u32 = 0;

    #[test]
    fn test_jhash_3vals() {
        assert_eq!(jhash_3vals(1, 2, 3, JHASH_INITVAL), 2757843189);
        assert_eq!(jhash_3vals(4, 5, 6, JHASH_INITVAL), 3701334656);

        assert_eq!(jhash_3vals(0, 0, 0, JHASH_INITVAL), 459859287);
        assert_eq!(
            jhash_3vals(u32::MAX, u32::MAX, u32::MAX, JHASH_INITVAL),
            1846109353
        );

        assert_eq!(jhash_3vals(1, 2, 3, 10), 453614296);
        assert_eq!(jhash_3vals(10, 20, 30, 5), 1448556389);
    }

    #[test]
    fn test_jhash_2vals() {
        assert_eq!(jhash_2vals(1, 2, JHASH_INITVAL), 2337044857);
        assert_eq!(jhash_2vals(3, 4, JHASH_INITVAL), 3842257880);
        assert_eq!(jhash_2vals(0, 0, JHASH_INITVAL), 1489077439);
        assert_eq!(jhash_2vals(u32::MAX, u32::MAX, JHASH_INITVAL), 1382321797);
        assert_eq!(jhash_2vals(1, 2, 10), 1474913524);
        assert_eq!(jhash_2vals(10, 20, 5), 1368945286);
    }

    #[test]
    fn test_jhash_1vals() {
        assert_eq!(jhash_1vals(1, JHASH_INITVAL), 1923623579);
        assert_eq!(jhash_1vals(5, JHASH_INITVAL), 4121471414);
        assert_eq!(jhash_1vals(0, JHASH_INITVAL), 76781240);
        assert_eq!(jhash_1vals(u32::MAX, JHASH_INITVAL), 2601633627);
        assert_eq!(jhash_1vals(1, 10), 1508237099);
        assert_eq!(jhash_1vals(10, 5), 2141486731);
    }

    #[test]
    fn test_jhash_u32_array() {
        assert_eq!(jhash_u32_array(&[1, 2, 3], JHASH_INITVAL), 2757843189);
        assert_eq!(jhash_u32_array(&[4, 5, 6, 7, 8], JHASH_INITVAL), 581654130);
        assert_eq!(jhash_u32_array(&[], JHASH_INITVAL), 3735928559);
        assert_eq!(jhash_u32_array(&[10], JHASH_INITVAL), 1030482289);
        assert_eq!(jhash_u32_array(&[10, 20], JHASH_INITVAL), 363923158);
        assert_eq!(
            jhash_u32_array(&[0, 1, 2, u32::MAX], JHASH_INITVAL),
            3019125658
        );
        assert_eq!(jhash_u32_array(&[1, 2, 3], 10), 453614296);
    }

    #[test]
    fn test_jhash_slice() {
        assert_eq!(jhash_slice(b"hello world", JHASH_INITVAL), 1252609637);
        assert_eq!(jhash_slice(b"12345", JHASH_INITVAL), 729031446);
        assert_eq!(jhash_slice(b"\n\t\r", JHASH_INITVAL), 483925400);
        assert_eq!(jhash_slice(b"", JHASH_INITVAL), 3735928559);

        let test_slices = &[
            b"12345".as_slice(),
            b"hello world hello world",
            b"\n",
            b"",
            b"\t\r\n",
        ];
        fn hash_slices(slices: &[&[u8]]) -> u32 {
            let mut hash: u32 = 0;
            for slice in slices {
                hash = jhash_slice(slice, hash);
            }
            hash
        }
        assert_eq!(hash_slices(test_slices), 3662697720);
        assert_eq!(hash_slices(&test_slices[0..4]), 230550114);
        assert_eq!(hash_slices(&test_slices[0..3]), 789588851);
        assert_eq!(hash_slices(&test_slices[0..2]), 1101610926);
        assert_eq!(hash_slices(&test_slices[0..1]), 729031446);
        assert_eq!(hash_slices(&[]), 0);
    }
}
