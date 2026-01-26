// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate ostd_pod;
use ostd_pod::{FromZeros, IntoBytes, Pod};

#[test]
fn pod_derive_simple() {
    #[repr(C)]
    #[derive(Pod, Debug, Clone, Copy, PartialEq)]
    struct S1 {
        a: u64,
        b: [u8; 8],
    }

    let s = S1 {
        a: 42,
        b: [1, 2, 3, 4, 5, 6, 7, 8],
    };
    let bytes = s.as_bytes();
    assert_eq!(bytes.len(), size_of::<S1>());

    let s2 = S1::from_bytes(bytes);
    assert_eq!(s, s2);
}

#[test]
fn pod_derive_generic() {
    #[repr(C)]
    #[derive(Pod, Clone, Copy, PartialEq, Debug)]
    struct Item<T: Pod> {
        value: T,
    }

    let item = Item { value: 5u64 };
    let bytes = item.as_bytes();
    assert_eq!(bytes.len(), size_of::<Item<u64>>());

    let item2 = Item::from_bytes(bytes);
    assert_eq!(item, item2);
}

#[test]
fn pod_derive_zeroed() {
    #[repr(C)]
    #[derive(Pod, Copy, Clone)]
    struct Data {
        x: u64,
        y: u64,
    }

    let zeroed = Data::new_zeroed();
    assert_eq!(zeroed.x, 0);
    assert_eq!(zeroed.y, 0);
}
