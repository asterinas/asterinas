// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{Infallible, VmReader};
use ostd_pod::IntoBytes;
use smoltcp::wire::IpRepr;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub(super) struct Be16([u8; 2]);

impl From<u16> for Be16 {
    fn from(value: u16) -> Self {
        Self(value.to_be_bytes())
    }
}

impl From<Be16> for u16 {
    fn from(value: Be16) -> Self {
        Self::from_be_bytes(value.0)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub(super) struct Be32([u8; 4]);

impl From<u32> for Be32 {
    fn from(value: u32) -> Self {
        Self(value.to_be_bytes())
    }
}

impl From<Be32> for u32 {
    fn from(value: Be32) -> Self {
        Self::from_be_bytes(value.0)
    }
}

#[derive(Debug)]
pub(super) struct Checksum(u16);

impl Checksum {
    pub(super) const fn new() -> Self {
        Checksum(0)
    }

    pub(super) fn with_bytes(self, bytes: &[u8]) -> Self {
        self.with_reader(VmReader::from(bytes))
    }

    pub(super) fn with_reader(self, reader: VmReader<Infallible>) -> Self {
        let v = do_checksum(reader);
        let (r, of) = self.0.overflowing_add(v);
        if of { Self(r + 1) } else { Self(r) }
    }

    pub(super) fn with_pseudo(self, ip_repr: &IpRepr, payload_len: usize) -> Self {
        match ip_repr {
            IpRepr::Ipv4(ipv4_repr) => {
                let pseudo_header = PseudoHeaderV4 {
                    src: ipv4_repr.src_addr.octets(),
                    dst: ipv4_repr.dst_addr.octets(),
                    zero: 0,
                    proto: ipv4_repr.next_header.into(),
                    len: (payload_len as u16).into(),
                };
                self.with_bytes(pseudo_header.as_bytes())
            }
            IpRepr::Ipv6(ipv6_repr) => {
                let pseudo_header = PseudoHeaderV6 {
                    src: ipv6_repr.src_addr.octets(),
                    dst: ipv6_repr.dst_addr.octets(),
                    len: (payload_len as u32).into(),
                    zero: [0; 3],
                    next: ipv6_repr.next_header.into(),
                };
                self.with_bytes(pseudo_header.as_bytes())
            }
        }
    }

    pub(super) fn finish(self) -> u16 {
        self.0
    }
}

fn do_checksum(mut reader: VmReader<Infallible>) -> u16 {
    let mut v0 = 0;
    let mut v1 = 0;
    let mut v2 = 0;
    let mut v3 = 0;

    fn add(a: u64, b: u64) -> u64 {
        let (c, of) = a.overflowing_add(b);
        if of { c + 1 } else { c }
    }

    // We can compute the checksum in native endianness. For more details,
    // see <https://datatracker.ietf.org/doc/html/rfc1071>.

    while reader.remain() >= size_of::<u64>() * 4 {
        let a0 = reader.read_val::<u64>().unwrap();
        let a1 = reader.read_val::<u64>().unwrap();
        let a2 = reader.read_val::<u64>().unwrap();
        let a3 = reader.read_val::<u64>().unwrap();

        v0 = add(v0, a0);
        v1 = add(v1, a1);
        v2 = add(v2, a2);
        v3 = add(v3, a3);
    }

    while let Ok(a0) = reader.read_val::<u64>() {
        v0 = add(v0, a0);
    }

    while let Ok(a0) = reader.read_val::<u16>() {
        v0 = add(v0, a0 as u64);
    }

    if let Ok(a0) = reader.read_val::<u8>() {
        let a0 = u16::from_ne_bytes([a0, 0]);
        v0 = add(v0, a0 as u64);
    }

    let mut v = add(add(v0, v1), add(v2, v3));
    v = (v >> 32) + (v as u32 as u64);
    v = (v >> 32) + (v as u32 as u64);
    v = (v >> 16) + (v as u16 as u64);
    v = (v >> 16) + (v as u16 as u64);

    debug_assert!(v <= u16::MAX as u64);
    v as u16
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct PseudoHeaderV4 {
    src: [u8; 4],
    dst: [u8; 4],
    zero: u8,
    proto: u8,
    len: Be16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct PseudoHeaderV6 {
    src: [u8; 16],
    dst: [u8; 16],
    len: Be32,
    zero: [u8; 3],
    next: u8,
}

#[cfg(ktest)]
mod test {
    use ostd::{mm::VmReader, prelude::ktest};

    #[ktest]
    fn checksum_matches_rfc_1071() {
        // See <https://datatracker.ietf.org/doc/html/rfc1071#section-3>.

        let rfc_example = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        let sum = super::Checksum::new()
            .with_reader(VmReader::from(&rfc_example[..]))
            .finish();
        assert_eq!(sum.to_ne_bytes(), [0xdd, 0xf2]);

        let rfc_example = [0x00, 0x01, 0xf2];
        let sum = super::Checksum::new()
            .with_reader(VmReader::from(&rfc_example[..]))
            .finish();
        assert_eq!(sum.to_ne_bytes(), [0xf2, 0x01]);
    }
}
