// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ostd::{
    prelude::ktest,
    task::{disable_preempt, Task},
};

use super::*;

macro_rules! n {
    ( $x:expr ) => {
        $x * 10
    };
}

fn init_continuous_with_arc<M>(xarray: &XArray<Arc<i32>, M>, item_num: i32) {
    for i in 0..item_num {
        let value = Arc::new(i);
        xarray.lock().store(i as u64, value);
    }
}

fn init_sparse_with_arc<M>(xarray: &XArray<Arc<i32>, M>, item_num: i32) {
    for i in 0..2 * item_num {
        if i % 2 == 0 {
            let value = Arc::new(i);
            xarray.lock().store(i as u64, value);
        }
    }
}

#[ktest]
fn store_continuous() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));
    let guard = disable_preempt();
    for i in 0..n!(100) {
        let value = xarray_arc.load(&guard, i as u64).unwrap();
        assert_eq!(*value.as_ref(), i);
    }
}

#[ktest]
fn store_sparse() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_sparse_with_arc(&xarray_arc, n!(100));

    let guard = disable_preempt();
    for i in 0..n!(100) {
        if i % 2 == 0 {
            let value = xarray_arc.load(&guard, i as u64).unwrap();
            assert_eq!(*value.as_ref(), i);
        }
    }
}

#[ktest]
fn store_overwrite() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));

    let mut locked_xarray = xarray_arc.lock();
    // Overwrite 20 at index 10.
    let value = Arc::new(20);
    locked_xarray.store(10, value);
    let v = locked_xarray.load(10).unwrap();
    assert_eq!(*v.as_ref(), 20);
    // Overwrite 40 at index 10.
    let value = Arc::new(40);
    locked_xarray.store(10, value);
    let v = locked_xarray.load(10).unwrap();
    assert_eq!(*v.as_ref(), 40);
}

#[ktest]
fn remove() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    assert!(xarray_arc.lock().remove(n!(1)).is_none());
    init_continuous_with_arc(&xarray_arc, n!(100));

    let mut locked_xarray = xarray_arc.lock();
    for i in 0..n!(100) {
        assert_eq!(*locked_xarray.remove(i as u64).unwrap().as_ref(), i);
        let value = locked_xarray.load(i as u64);
        assert!(value.is_none());
        assert!(locked_xarray.remove(i as u64).is_none());
    }
}

#[ktest]
fn cursor_load() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));

    let guard = disable_preempt();
    let mut cursor = xarray_arc.cursor(&guard, 0);

    for i in 0..n!(100) {
        let value = cursor.load().unwrap();
        assert_eq!(*value.as_ref(), i);
        cursor.next();
    }

    cursor.reset_to(n!(200));
    assert!(cursor.load().is_none());
}

#[ktest]
fn cursor_load_very_sparse() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    let mut locked_xarray = xarray_arc.lock();
    locked_xarray.store(0, Arc::new(1));
    locked_xarray.store(n!(100), Arc::new(2));

    let mut cursor = locked_xarray.cursor(0);
    assert_eq!(*cursor.load().unwrap().as_ref(), 1);
    for _ in 0..n!(100) {
        cursor.next();
    }
    assert_eq!(*cursor.load().unwrap().as_ref(), 2);
}

#[ktest]
fn cursor_store_continuous() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    let mut locked_xarray = xarray_arc.lock();
    let mut cursor = locked_xarray.cursor_mut(0);

    for i in 0..n!(100) {
        let value = Arc::new(i);
        cursor.store(value);
        cursor.next();
    }

    for i in 0..n!(100) {
        let value = locked_xarray.load(i as u64).unwrap();
        assert_eq!(*value.as_ref(), i);
    }
}

#[ktest]
fn cursor_store_sparse() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    let mut locked_xarray = xarray_arc.lock();
    let mut cursor = locked_xarray.cursor_mut(0);

    for i in 0..n!(100) {
        if i % 2 == 0 {
            let value = Arc::new(i);
            cursor.store(value);
        }
        cursor.next();
    }

    for i in 0..n!(100) {
        if i % 2 == 0 {
            let value = locked_xarray.load(i as u64).unwrap();
            assert_eq!(*value.as_ref(), i);
        }
    }
}

#[ktest]
fn set_mark() {
    let xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));

    let mut locked_xarray = xarray_arc.lock();
    let mut cursor = locked_xarray.cursor_mut(n!(10));
    cursor.set_mark(XMark::Mark0).unwrap();
    cursor.set_mark(XMark::Mark1).unwrap();
    cursor.reset_to(n!(20));
    cursor.set_mark(XMark::Mark1).unwrap();

    cursor.reset_to(n!(10));
    let value1_mark0 = cursor.is_marked(XMark::Mark0);
    let value1_mark1 = cursor.is_marked(XMark::Mark1);

    cursor.reset_to(n!(20));
    let value2_mark0 = cursor.is_marked(XMark::Mark0);
    let value2_mark1 = cursor.is_marked(XMark::Mark1);

    cursor.reset_to(n!(30));
    let value3_mark1 = cursor.is_marked(XMark::Mark1);

    assert!(value1_mark0);
    assert!(value1_mark1);
    assert!(!value2_mark0);
    assert!(value2_mark1);
    assert!(!value3_mark1);
}

#[ktest]
fn unset_mark() {
    let xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));

    let mut locked_xarray = xarray_arc.lock();
    let mut cursor = locked_xarray.cursor_mut(n!(10));
    cursor.set_mark(XMark::Mark0).unwrap();
    cursor.set_mark(XMark::Mark1).unwrap();

    cursor.unset_mark(XMark::Mark0).unwrap();
    cursor.unset_mark(XMark::Mark2).unwrap();

    let value1_mark0 = cursor.is_marked(XMark::Mark0);
    let value1_mark2 = cursor.is_marked(XMark::Mark2);
    assert!(!value1_mark0);
    assert!(!value1_mark2);
}

#[ktest]
fn mark_overflow() {
    let xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));
    let mut locked_xarray = xarray_arc.lock();
    let mut cursor = locked_xarray.cursor_mut(n!(200));

    assert!(cursor.set_mark(XMark::Mark1).is_err());
    assert!(!cursor.is_marked(XMark::Mark1));
}

#[ktest]
fn box_operate() {
    let xarray_box: XArray<Box<i32>> = XArray::new();
    let mut locked_xarray = xarray_box.lock();
    let mut cursor_mut = locked_xarray.cursor_mut(0);
    for i in 0..n!(100) {
        if i % 2 == 0 {
            cursor_mut.store(Box::new(i * 2));
        }
        cursor_mut.next();
    }

    cursor_mut.reset_to(0);
    for i in 0..n!(100) {
        if i % 2 == 0 {
            assert_eq!(*cursor_mut.load().unwrap().as_ref(), i * 2);
        } else {
            assert!(cursor_mut.load().is_none());
        }
        cursor_mut.next();
    }

    let mut cursor = locked_xarray.cursor(0);
    for i in 0..n!(100) {
        if i % 2 == 0 {
            assert_eq!(*cursor.load().unwrap().as_ref(), i * 2);
        } else {
            assert!(cursor.load().is_none());
        }
        cursor.next();
    }
}

#[ktest]
fn range() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    for i in 0..n!(100) {
        let value = Arc::new(i * 2);
        xarray_arc.lock().store((i * 2) as u64, value);
    }

    let mut count = 0;
    let guard = disable_preempt();
    for (index, item) in xarray_arc.range(&guard, n!(10)..n!(20)) {
        assert_eq!(*item.as_ref() as u64, index);
        count += 1;
    }
    assert_eq!(count, n!(5));
}

#[ktest]
fn load_after_clear() {
    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&xarray_arc, n!(100));

    let guard = disable_preempt();
    let mut cursor = xarray_arc.cursor(&guard, 100);
    let mut locked_xarray = xarray_arc.lock();

    let value = cursor.load().unwrap();
    assert_eq!(*value.as_ref(), 100);

    // Read the old data.
    locked_xarray.clear();
    let value = cursor.load().unwrap();
    assert_eq!(*value.as_ref(), 100);

    // Read the new data.
    cursor.reset();
    assert!(cursor.load().is_none());
}

static TEST_LEAKAGE: AtomicBool = AtomicBool::new(false);
static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

impl<P: NonNullPtr + Send + Sync> Drop for node::XNode<P> {
    fn drop(&mut self) {
        if TEST_LEAKAGE.load(Ordering::Relaxed) {
            DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[ktest]
fn no_leakage() {
    // FIXME: This doesn't work on multi-processor. We can't yet schedule tasks
    // to other CPUs with the OSTD's default scheduler.
    fn finish_grace_period() {
        let task = || {};
        let _ = ostd::task::TaskOptions::new(task).data(()).spawn();
        Task::yield_now();
    }

    // Drop the nodes created by the previous tests.
    finish_grace_period();
    TEST_LEAKAGE.store(true, Ordering::Relaxed);

    let xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_sparse_with_arc(&xarray_arc, (SLOT_SIZE * SLOT_SIZE / 2 + 1) as i32);
    drop(xarray_arc);

    // Drop the nodes created in the test.
    finish_grace_period();
    TEST_LEAKAGE.store(false, Ordering::Relaxed);

    let count = DROP_COUNT.load(Ordering::Relaxed);
    // layer 3: 1
    // layer 2: 1 + 1
    // layer 1: 64 + 1
    let expected = 68;
    assert_eq!(count, expected);
}
