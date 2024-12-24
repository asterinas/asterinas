// SPDX-License-Identifier: MPL-2.0

//! This module implements hash algorithms from Bob Jenkins.
//!
//! See: www.burtleburtle.net/bob/hash/doobs.html
//!
//! Hash functions in this module supports producing 32-bit value for hashtable lookup.
//! The [Linux kernel's jhash](https://github.com/torvalds/linux/blob/master/include/linux/jhash.h)
//! is a bit different from the original version,
//! This module reimplements the Linux kernel's version.
//!

/// A random chosen value
const JHASH_INITVAL: u32 = 0xdeadbeef;

/// Hashes exactly 3 u32 values
pub fn jhash_3vals(a: u32, b: u32, c: u32, initval: u32) -> u32 {
    jhash_3vals_inner(
        a,
        b,
        c,
        initval.wrapping_add(JHASH_INITVAL).wrapping_add(3 << 2),
    )
}

/// Hashes exactly 2 u32 values
pub fn jhash_2vals(a: u32, b: u32, initval: u32) -> u32 {
    jhash_3vals_inner(
        a,
        b,
        0,
        initval.wrapping_add(JHASH_INITVAL).wrapping_add(2 << 2),
    )
}

/// Hashes exactly 1 u32 values
pub fn jhash_1vals(a: u32, initval: u32) -> u32 {
    jhash_3vals_inner(
        a,
        0,
        0,
        initval.wrapping_add(JHASH_INITVAL).wrapping_add(1 << 2),
    )
}

/// Hashes a u32 array.
pub fn jhash_array(array: &[u32], initval: u32) -> u32 {
    let mut length = array.len() as u32;
    let mut index = 0;

    let mut a: u32 = JHASH_INITVAL
        .wrapping_add(length << 2)
        .wrapping_add(initval);
    let mut b: u32 = a;
    let mut c: u32 = a;

    // Handle most values except last 3 values
    while length > 3 {
        a = a.wrapping_add(array[index]);
        b = b.wrapping_add(array[index + 1]);
        c = c.wrapping_add(array[index + 2]);

        (a, b, c) = jhash_mix(a, b, c);
        length -= 3;
        index += 3;
    }

    // Handle the last 3 values

    debug_assert!(length <= 3);

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

fn jhash_3vals_inner(mut a: u32, mut b: u32, mut c: u32, initval: u32) -> u32 {
    a = a.wrapping_add(initval);
    b = b.wrapping_add(initval);
    c = c.wrapping_add(initval);

    jhash_final(a, b, c)
}

/// Mixes three 32-bit values into a final u32 value
fn jhash_final(mut a: u32, mut b: u32, mut c: u32) -> u32 {
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

///  Mixes three 32-bit values in a reversible manner.
fn jhash_mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
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

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    const JHASH_INITVAL: u32 = 0;

    #[ktest]
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

    #[ktest]
    fn test_jhash_2vals() {
        assert_eq!(jhash_2vals(1, 2, JHASH_INITVAL), 2337044857);
        assert_eq!(jhash_2vals(3, 4, JHASH_INITVAL), 3842257880);
        assert_eq!(jhash_2vals(0, 0, JHASH_INITVAL), 1489077439);
        assert_eq!(jhash_2vals(u32::MAX, u32::MAX, JHASH_INITVAL), 1382321797);
        assert_eq!(jhash_2vals(1, 2, 10), 1474913524);
        assert_eq!(jhash_2vals(10, 20, 5), 1368945286);
    }

    #[ktest]
    fn test_jhash_1vals() {
        assert_eq!(jhash_1vals(1, JHASH_INITVAL), 1923623579);
        assert_eq!(jhash_1vals(5, JHASH_INITVAL), 4121471414);
        assert_eq!(jhash_1vals(0, JHASH_INITVAL), 76781240);
        assert_eq!(jhash_1vals(u32::MAX, JHASH_INITVAL), 2601633627);
        assert_eq!(jhash_1vals(1, 10), 1508237099);
        assert_eq!(jhash_1vals(10, 5), 2141486731);
    }

    #[ktest]
    fn test_jhash_array() {
        assert_eq!(jhash_array(&[1, 2, 3], JHASH_INITVAL), 2757843189);
        assert_eq!(jhash_array(&[4, 5, 6, 7, 8], JHASH_INITVAL), 581654130);
        assert_eq!(jhash_array(&[], JHASH_INITVAL), 3735928559);
        assert_eq!(jhash_array(&[10], JHASH_INITVAL), 1030482289);
        assert_eq!(jhash_array(&[10, 20], JHASH_INITVAL), 363923158);
        assert_eq!(jhash_array(&[0, 1, 2, u32::MAX], JHASH_INITVAL), 3019125658);
        assert_eq!(jhash_array(&[1, 2, 3], 10), 453614296);
    }
}
