// SPDX-License-Identifier: MPL-2.0

//! Directory-entry name hashing for htree (`dir_index`), ported from Linux 6.6
//! `fs/ext4/hash.c` (`ext4fs_dirhash`).
//!
//! htree keys each directory entry by a hash of its name; `dx_probe` binary-
//! searches the index by this value. The hash MUST match Linux bit-for-bit or a
//! lookup lands on the wrong leaf and misses the name. Three versions are
//! supported — the legacy hack hash,
//! a cut-down half-MD4, and TEA — each in a signed- and unsigned-`char` variant
//! (the historical ambiguity of `char`'s signedness on the hashed bytes).
//! `siphash` (casefold) is not supported.
//!
//! All arithmetic is 32-bit wrapping, exactly as the C wraps `__u32`.

/// Hash versions (`DX_HASH_*`, `ext4.h`). The `*_UNSIGNED` variants hash the
/// name bytes as unsigned rather than signed `char`.
pub(super) const DX_HASH_LEGACY: u8 = 0;
pub(super) const DX_HASH_HALF_MD4: u8 = 1;
pub(super) const DX_HASH_TEA: u8 = 2;
pub(super) const DX_HASH_LEGACY_UNSIGNED: u8 = 3;
pub(super) const DX_HASH_HALF_MD4_UNSIGNED: u8 = 4;
pub(super) const DX_HASH_TEA_UNSIGNED: u8 = 5;

/// The largest 32-bit hash value, reserved as the readdir end-of-file sentinel
/// (`EXT4_HTREE_EOF_32BIT`); a name hashing to it is nudged down by one.
const EXT4_HTREE_EOF_32BIT: u32 = (1 << 31) - 1;

/// The result of hashing a directory entry name: `hash` keys the htree index;
/// `minor_hash` disambiguates collisions in the readdir cursor (0 for the
/// 32-bit legacy hash).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DirHash {
    pub hash: u32,
    /// Feeds the hash-ordered readdir cursor, which stays linear (path C); the
    /// lookup path keys only on `hash`. Kept for the on-disk hash's full result.
    pub minor_hash: u32,
}

const DELTA: u32 = 0x9E37_79B9;

/// TEA, 16 rounds (Linux `TEA_transform`).
fn tea_transform(buf: &mut [u32; 4], input: &[u32; 4]) {
    let mut sum: u32 = 0;
    let (mut b0, mut b1) = (buf[0], buf[1]);
    let (a, b, c, d) = (input[0], input[1], input[2], input[3]);

    for _ in 0..16 {
        sum = sum.wrapping_add(DELTA);
        b0 = b0.wrapping_add(
            ((b1 << 4).wrapping_add(a)) ^ b1.wrapping_add(sum) ^ ((b1 >> 5).wrapping_add(b)),
        );
        b1 = b1.wrapping_add(
            ((b0 << 4).wrapping_add(c)) ^ b0.wrapping_add(sum) ^ ((b0 >> 5).wrapping_add(d)),
        );
    }

    buf[0] = buf[0].wrapping_add(b0);
    buf[1] = buf[1].wrapping_add(b1);
}

// MD4 basic functions (selection / majority / parity).
fn md4_f(x: u32, y: u32, z: u32) -> u32 {
    z ^ (x & (y ^ z))
}
fn md4_g(x: u32, y: u32, z: u32) -> u32 {
    (x & y).wrapping_add((x ^ y) & z)
}
fn md4_h(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

/// One MD4 round step (`ROUND`): `a += f(b,c,d) + x; a = rol32(a, s)`.
fn md4_round(f: fn(u32, u32, u32) -> u32, a: u32, b: u32, c: u32, d: u32, x: u32, s: u32) -> u32 {
    a.wrapping_add(f(b, c, d)).wrapping_add(x).rotate_left(s)
}

/// Cut-down MD4, returning the "most hashed" word (Linux `half_md4_transform`).
fn half_md4_transform(buf: &mut [u32; 4], input: &[u32; 8]) -> u32 {
    // Octal round constants, verbatim from `hash.c` (K2/K3).
    const K1: u32 = 0;
    const K2: u32 = 0o13240474631;
    const K3: u32 = 0o15666365641;

    let (mut a, mut b, mut c, mut d) = (buf[0], buf[1], buf[2], buf[3]);

    // Round 1 (F).
    a = md4_round(md4_f, a, b, c, d, input[0].wrapping_add(K1), 3);
    d = md4_round(md4_f, d, a, b, c, input[1].wrapping_add(K1), 7);
    c = md4_round(md4_f, c, d, a, b, input[2].wrapping_add(K1), 11);
    b = md4_round(md4_f, b, c, d, a, input[3].wrapping_add(K1), 19);
    a = md4_round(md4_f, a, b, c, d, input[4].wrapping_add(K1), 3);
    d = md4_round(md4_f, d, a, b, c, input[5].wrapping_add(K1), 7);
    c = md4_round(md4_f, c, d, a, b, input[6].wrapping_add(K1), 11);
    b = md4_round(md4_f, b, c, d, a, input[7].wrapping_add(K1), 19);

    // Round 2 (G).
    a = md4_round(md4_g, a, b, c, d, input[1].wrapping_add(K2), 3);
    d = md4_round(md4_g, d, a, b, c, input[3].wrapping_add(K2), 5);
    c = md4_round(md4_g, c, d, a, b, input[5].wrapping_add(K2), 9);
    b = md4_round(md4_g, b, c, d, a, input[7].wrapping_add(K2), 13);
    a = md4_round(md4_g, a, b, c, d, input[0].wrapping_add(K2), 3);
    d = md4_round(md4_g, d, a, b, c, input[2].wrapping_add(K2), 5);
    c = md4_round(md4_g, c, d, a, b, input[4].wrapping_add(K2), 9);
    b = md4_round(md4_g, b, c, d, a, input[6].wrapping_add(K2), 13);

    // Round 3 (H).
    a = md4_round(md4_h, a, b, c, d, input[3].wrapping_add(K3), 3);
    d = md4_round(md4_h, d, a, b, c, input[7].wrapping_add(K3), 9);
    c = md4_round(md4_h, c, d, a, b, input[2].wrapping_add(K3), 11);
    b = md4_round(md4_h, b, c, d, a, input[6].wrapping_add(K3), 15);
    a = md4_round(md4_h, a, b, c, d, input[1].wrapping_add(K3), 3);
    d = md4_round(md4_h, d, a, b, c, input[5].wrapping_add(K3), 9);
    c = md4_round(md4_h, c, d, a, b, input[0].wrapping_add(K3), 11);
    b = md4_round(md4_h, b, c, d, a, input[4].wrapping_add(K3), 15);

    buf[0] = buf[0].wrapping_add(a);
    buf[1] = buf[1].wrapping_add(b);
    buf[2] = buf[2].wrapping_add(c);
    buf[3] = buf[3].wrapping_add(d);

    buf[1] // "most hashed" word
}

/// The old legacy hash (Linux `dx_hack_hash_{signed,unsigned}`). `signed`
/// selects how each name byte is widened before the multiply.
fn dx_hack_hash(name: &[u8], signed: bool) -> u32 {
    let (mut hash0, mut hash1): (u32, u32) = (0x12a3_fe2d, 0x37ab_e8f9);
    for &byte in name {
        let widened = if signed {
            (byte as i8 as i32).wrapping_mul(7_152_373) as u32
        } else {
            (byte as u32).wrapping_mul(7_152_373)
        };
        let mut hash = hash1.wrapping_add(hash0 ^ widened);
        if hash & 0x8000_0000 != 0 {
            hash = hash.wrapping_sub(0x7fff_ffff);
        }
        hash1 = hash0;
        hash0 = hash;
    }
    hash0 << 1
}

/// Packs up to `num` 32-bit words from `msg` (Linux `str2hashbuf_{signed,unsigned}`).
fn str2hashbuf(msg: &[u8], buf: &mut [u32; 8], mut num: usize, signed: bool) {
    let len_full = msg.len();
    let mut pad = (len_full as u32) | ((len_full as u32) << 8);
    pad |= pad << 16;

    let mut val = pad;
    let len = len_full.min(num * 4);
    let mut out = 0usize;
    for (i, &byte) in msg[..len].iter().enumerate() {
        let widened = if signed {
            byte as i8 as i32 as u32
        } else {
            byte as u32
        };
        val = widened.wrapping_add(val << 8);
        if i % 4 == 3 {
            buf[out] = val;
            out += 1;
            val = pad;
            num -= 1;
        }
    }
    // `if (--num >= 0) *buf++ = val;` then `while (--num >= 0) *buf++ = pad;`
    if num >= 1 {
        buf[out] = val;
        out += 1;
        num -= 1;
    }
    while num >= 1 {
        buf[out] = pad;
        out += 1;
        num -= 1;
    }
}

/// Hashes directory entry `name` under `version` and `seed` (`s_hash_seed`),
/// returning `None` for an unsupported version (e.g. `siphash`). Mirrors Linux
/// `ext4fs_dirhash`: default seed unless `seed` has a nonzero word, the
/// version-specific transform, then `hash &= ~1` and the EOF nudge.
pub(super) fn ext4fs_dirhash(name: &[u8], version: u8, seed: &[u32; 4]) -> Option<DirHash> {
    // Default seed for the checksum functions, overridden by a nonzero `seed`.
    let mut buf: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    if seed.iter().any(|&w| w != 0) {
        buf = *seed;
    }

    let mut minor_hash = 0u32;
    let hash = match version {
        DX_HASH_LEGACY_UNSIGNED => dx_hack_hash(name, false),
        DX_HASH_LEGACY => dx_hack_hash(name, true),
        DX_HASH_HALF_MD4 | DX_HASH_HALF_MD4_UNSIGNED => {
            let signed = version == DX_HASH_HALF_MD4;
            let mut input = [0u32; 8];
            let mut p = name;
            loop {
                str2hashbuf(p, &mut input, 8, signed);
                half_md4_transform(&mut buf, &input);
                if p.len() <= 32 {
                    break;
                }
                p = &p[32..];
            }
            minor_hash = buf[2];
            buf[1]
        }
        DX_HASH_TEA | DX_HASH_TEA_UNSIGNED => {
            let signed = version == DX_HASH_TEA;
            let mut input8 = [0u32; 8];
            let mut tea_in = [0u32; 4];
            let mut p = name;
            loop {
                str2hashbuf(p, &mut input8, 4, signed);
                tea_in.copy_from_slice(&input8[..4]);
                tea_transform(&mut buf, &tea_in);
                if p.len() <= 16 {
                    break;
                }
                p = &p[16..];
            }
            minor_hash = buf[1];
            buf[0]
        }
        _ => return None, // siphash / unknown
    };

    let mut hash = hash & !1;
    if hash == EXT4_HTREE_EOF_32BIT << 1 {
        hash = (EXT4_HTREE_EOF_32BIT - 1) << 1;
    }
    Some(DirHash { hash, minor_hash })
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    // Reference vectors captured from e2fsprogs 1.47 `debugfs htree_dump` on real
    // `mke2fs -O dir_index` images (each with its own random `s_hash_seed`). The
    // stored dx_root hash_version selects the transform; these images set
    // SIGNED_HASH (s_flags bit 0), so the signed variants apply.

    /// half_md4 (the mkfs default), seed from the sample image.
    const MD4_SEED: [u32; 4] = [0xaec4_c740, 0x834c_e862, 0x09b5_3288, 0x34dc_d3db];

    #[ktest]
    fn half_md4_matches_e2fsprogs() {
        let h = ext4fs_dirhash(b"file0", DX_HASH_HALF_MD4, &MD4_SEED).unwrap();
        assert_eq!(h.hash, 0x1c3d_2670);
        assert_eq!(h.minor_hash, 0xfca4_88c5);
        assert_eq!(
            ext4fs_dirhash(b"file3", DX_HASH_HALF_MD4, &MD4_SEED)
                .unwrap()
                .hash,
            0x4f04_13ba
        );
        // The stored hash is always even (`hash & ~1`).
        assert_eq!(h.hash & 1, 0);
    }

    /// A name with a high byte (0xE9) distinguishes the signed variant, which
    /// this image uses, from the unsigned one.
    #[ktest]
    fn half_md4_signed_high_byte() {
        let h = ext4fs_dirhash(b"x\xe9y", DX_HASH_HALF_MD4, &MD4_SEED).unwrap();
        assert_eq!(h.hash, 0xb8a7_e90a);
        assert_eq!(h.minor_hash, 0x3b68_6309);
        // The unsigned variant differs on a high byte (no Linux vector, just the
        // sign-widening path being distinct).
        let u = ext4fs_dirhash(b"x\xe9y", DX_HASH_HALF_MD4_UNSIGNED, &MD4_SEED).unwrap();
        assert_ne!(u.hash, h.hash);
        // ASCII names are identical across the signedness split.
        assert_eq!(
            ext4fs_dirhash(b"file0", DX_HASH_HALF_MD4_UNSIGNED, &MD4_SEED)
                .unwrap()
                .hash,
            0x1c3d_2670
        );
    }

    #[ktest]
    fn tea_matches_e2fsprogs() {
        let seed = [0x79e2_f44a, 0x9449_4804, 0xd0aa_bea6, 0x8275_3d4e];
        let h = ext4fs_dirhash(b"file0", DX_HASH_TEA, &seed).unwrap();
        assert_eq!(h.hash, 0xd483_fcc4);
        assert_eq!(h.minor_hash, 0x2166_1f2d);
    }

    #[ktest]
    fn legacy_matches_e2fsprogs() {
        let seed = [0x5001_7978, 0xb84a_f9d1, 0x3638_94a3, 0xe9cc_f611];
        let h = ext4fs_dirhash(b"file0", DX_HASH_LEGACY, &seed).unwrap();
        assert_eq!(h.hash, 0x9f12_3cc8);
        assert_eq!(h.minor_hash, 0); // 32-bit legacy hash carries no minor hash
    }

    #[ktest]
    fn unsupported_version_is_none() {
        assert!(ext4fs_dirhash(b"file0", 6 /* siphash */, &MD4_SEED).is_none());
    }
}
