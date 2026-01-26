// SPDX-License-Identifier: MPL-2.0

use ostd_pod::{FromZeros, IntoBytes, Pod, pod_union};

#[test]
fn union_roundtrip_from_bytes() {
    #[repr(C)]
    #[pod_union]
    #[derive(Copy, Clone)]
    union U1 {
        a: u32,
        b: u64,
    }

    let bytes: [u8; 8] = [0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11];
    let u = U1::from_bytes(&bytes);

    assert_eq!(u.as_bytes(), &bytes);
    assert_eq!(*u.b(), 0x1122_3344_5566_7788u64);
}

#[test]
fn union_field_view_through_bytes() {
    #[repr(C)]
    #[pod_union]
    #[derive(Copy, Clone)]
    union U2 {
        a: u64,
        b: [u8; 8],
    }

    let mut u = U2::new_zeroed();
    *u.b_mut() = [1, 2, 3, 4, 5, 6, 7, 8];
    let bytes = u.as_bytes();
    assert_eq!(bytes, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(*u.a(), 0x0807_0605_0403_0201u64);
}

#[test]
fn union_mutable_accessor() {
    #[repr(C)]
    #[pod_union]
    #[derive(Copy, Clone)]
    union U3 {
        x: u32,
        y: [u8; 8],
    }

    let mut u = U3::new_zeroed();

    // Modify field through mutable accessor
    *u.x_mut() = 0xAABBCCDD;
    assert_eq!(*u.x(), 0xAABBCCDD);
}
