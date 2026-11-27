// SPDX-License-Identifier: MPL-2.0

use super::{allocator::FrameAllocOptions, *};
use crate::{
    impl_frame_meta_for, impl_untyped_frame_meta_for,
    mm::{HasPaddrRange, Split, io::util::HasVmReaderWriter},
    prelude::*,
};

/// Typed mock metadata struct for testing
#[derive(Debug, Default)]
struct MockFrameMeta {
    value: u32,
}
impl_frame_meta_for!(MockFrameMeta);

/// Untyped mock metadata struct for testing
#[derive(Debug, Default)]
struct MockUFrameMeta {
    value: u32,
}
impl_untyped_frame_meta_for!(MockUFrameMeta);

// Frame allocation and management tests
mod frame {
    use super::*;

    #[ktest]
    fn frame_allocation() {
        let meta = MockFrameMeta { value: 42 };
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(meta)
            .expect("Failed to allocate single frame");
        assert_eq!(frame.meta().value, 42);
        assert_eq!(frame.reference_count(), 1);
        assert_eq!(frame.size(), PAGE_SIZE);
        assert_eq!(frame.map_level(), 1);
    }

    #[ktest]
    fn frame_clone() {
        let meta = MockFrameMeta { value: 42 };
        let frame1 = FrameAllocOptions::new()
            .alloc_frame_with(meta)
            .expect("Failed to allocate single frame");
        let frame2 = frame1.clone();
        assert_eq!(frame1.paddr(), frame2.paddr());
        assert_eq!(frame1.meta().value, frame2.meta().value);
        assert_eq!(frame1.reference_count(), 2);
        assert_eq!(frame2.reference_count(), 2);
    }

    #[ktest]
    fn frame_drop() {
        let metadata = MockFrameMeta { value: 42 };
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(metadata)
            .expect("Failed to allocate single frame");
        let ref_count_before = frame.reference_count();
        let paddr_before = frame.paddr();
        assert_eq!(ref_count_before, 1);
        drop(frame);
        let new_frame = FrameAllocOptions::new()
            .alloc_frame_with(MockFrameMeta { value: 42 })
            .expect("Failed to allocate single frame");
        assert_eq!(new_frame.paddr(), paddr_before);
        assert_eq!(new_frame.reference_count(), 1);
        assert_eq!(new_frame.meta().value, 42);
    }

    #[ktest]
    fn frame_to_uframe() {
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(MockUFrameMeta { value: 42 })
            .unwrap();
        let uframe: UFrame = frame.into();
        assert_eq!(uframe.size(), PAGE_SIZE);
    }

    #[ktest]
    fn frame_conversions() {
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(MockFrameMeta { value: 42 })
            .unwrap();
        let dyn_frame: Frame<dyn AnyFrameMeta> = frame.into();
        assert!(!dyn_frame.dyn_meta().is_untyped());
        let result: Result<Frame<MockFrameMeta>, _> = Frame::try_from(dyn_frame);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().meta().value, 42);
    }
}

// Frame linked list tests
mod linked_list {
    use super::*;
    use crate::mm::frame::linked_list::{Link, LinkedList};

    #[ktest]
    fn linked_list_push_pop() {
        let alloc_options = FrameAllocOptions::new();
        let frame1 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 1 }))
            .unwrap();
        let frame2 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 2 }))
            .unwrap();
        let frame3 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 3 }))
            .unwrap();

        let mut list = LinkedList::new();
        assert!(list.is_empty());
        list.push_front(frame1.try_into().unwrap());
        // 1
        list.push_front(frame2.try_into().unwrap());
        // 2 1
        list.push_back(frame3.try_into().unwrap());
        // 2 1 3
        assert_eq!(list.size(), 3);

        let mut cursor = list.cursor_front_mut();
        // *2 1 3 ()
        assert_eq!(cursor.current_meta().unwrap().value, 2);
        cursor.move_next();
        // 2 *1 3 ()
        assert_eq!(cursor.current_meta().unwrap().value, 1);
        cursor.move_next();
        // 2 1 *3 ()
        assert_eq!(cursor.current_meta().unwrap().value, 3);
        cursor.move_next();
        // 2 1 3 *()
        assert!(cursor.current_meta().is_none());

        assert_eq!(cursor.as_list().size(), 3);

        assert_eq!(list.pop_front().unwrap().meta().value, 2);
        assert_eq!(list.size(), 2);
        // 1 3
        assert_eq!(list.pop_back().unwrap().meta().value, 3);
        assert_eq!(list.size(), 1);
        // 1
        assert_eq!(list.pop_front().unwrap().meta().value, 1);
        assert_eq!(list.size(), 0);
    }

    #[ktest]
    fn linked_list_cursor_at() {
        let alloc_options = FrameAllocOptions::new();
        let frame1 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 1 }))
            .unwrap();
        let frame1_addr = frame1.paddr();
        let frame2 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 2 }))
            .unwrap();
        let frame2_addr = frame2.paddr();
        let frame3 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 3 }))
            .unwrap();
        let frame3_addr = frame3.paddr();

        let frame_outside = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 4 }))
            .unwrap();

        let mut list = LinkedList::new();
        list.push_front(frame1.try_into().unwrap());
        list.push_front(frame2.try_into().unwrap());
        list.push_front(frame3.try_into().unwrap());

        assert!(!list.contains(frame_outside.paddr()));
        assert!(list.cursor_mut_at(frame_outside.paddr()).is_none());

        assert!(list.contains(frame1_addr));
        assert!(list.contains(frame2_addr));
        assert!(list.contains(frame3_addr));

        let mut cursor = list.cursor_mut_at(frame1_addr).unwrap();
        assert_eq!(cursor.current_meta().unwrap().value, 1);
        let mut cursor = list.cursor_mut_at(frame2_addr).unwrap();
        assert_eq!(cursor.current_meta().unwrap().value, 2);
        let mut cursor = list.cursor_mut_at(frame3_addr).unwrap();
        assert_eq!(cursor.current_meta().unwrap().value, 3);
    }

    #[ktest]
    fn linked_list_cursor_ops() {
        let alloc_options = FrameAllocOptions::new();
        let frame1 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 1 }))
            .unwrap();
        let frame2 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 2 }))
            .unwrap();
        let frame3 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 3 }))
            .unwrap();
        let frame4 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 4 }))
            .unwrap();
        let frame5 = alloc_options
            .alloc_frame_with(Link::new(MockUFrameMeta { value: 5 }))
            .unwrap();

        let mut list = LinkedList::new();
        assert!(list.is_empty());
        list.push_front(frame1.try_into().unwrap());
        // 1 *()
        list.push_front(frame2.try_into().unwrap());
        // 2 *1 ()
        assert_eq!(list.size(), 2);

        let mut cursor = list.cursor_front_mut();
        // *2 1 ()
        assert_eq!(cursor.current_meta().unwrap().value, 2);
        cursor.move_next();
        // 2 *1 ()
        assert_eq!(cursor.current_meta().unwrap().value, 1);
        cursor.insert_before(frame3.try_into().unwrap());
        // 2 3 *1 ()
        assert_eq!(cursor.current_meta().unwrap().value, 1);
        cursor.insert_before(frame4.try_into().unwrap());
        // 2 3 4 *1 ()
        assert_eq!(cursor.current_meta().unwrap().value, 1);
        cursor.move_next();
        // 2 3 4 1 *()
        assert!(cursor.current_meta().is_none());
        cursor.insert_before(frame5.try_into().unwrap());
        // 2 3 4 1 5 *()
        assert!(cursor.current_meta().is_none());

        assert_eq!(cursor.as_list().size(), 5);

        let mut cursor = list.cursor_front_mut();
        assert_eq!(cursor.current_meta().unwrap().value, 2);
        cursor.move_next();
        assert_eq!(cursor.current_meta().unwrap().value, 3);
        cursor.move_next();
        assert_eq!(cursor.current_meta().unwrap().value, 4);
        cursor.move_next();
        assert_eq!(cursor.current_meta().unwrap().value, 1);
        cursor.move_next();
        assert_eq!(cursor.current_meta().unwrap().value, 5);
        cursor.move_next();
        assert!(cursor.current_meta().is_none());
        // 2 3 4 1 5 *()

        cursor.move_prev();
        // 2 3 4 1 *5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 5);
        cursor.move_prev();
        // 2 3 4 *1 5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 1);

        let frame1 = cursor.take_current().unwrap();
        // 2 3 4 *5 ()
        assert_eq!(frame1.meta().value, 1);
        assert_eq!(cursor.current_meta().unwrap().value, 5);
        cursor.move_next();
        // 2 3 4 5 *()
        assert!(cursor.current_meta().is_none());
        cursor.move_prev();
        // 2 3 4 *5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 5);
        cursor.move_prev();
        // 2 3 *4 5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 4);
        cursor.move_prev();
        // 2 *3 4 5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 3);
        cursor.move_prev();
        // *2 3 4 5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 2);

        let frame2 = cursor.take_current().unwrap();
        // *3 4 5 ()
        assert_eq!(frame2.meta().value, 2);
        assert_eq!(cursor.current_meta().unwrap().value, 3);
        cursor.move_next();
        // 3 *4 5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 4);
        cursor.move_next();
        // 3 4 *5 ()
        assert_eq!(cursor.current_meta().unwrap().value, 5);
        // 3 4 *()
        let frame5 = cursor.take_current().unwrap();
        assert_eq!(frame5.meta().value, 5);
        assert!(cursor.current_meta().is_none());

        assert_eq!(cursor.as_list().size(), 2);
    }
}

// Segment tests
mod segment {
    use super::*;
    use crate::{Error, mm::USegment};

    #[ktest]
    fn segment_creation() {
        let range = 512 * PAGE_SIZE..1024 * PAGE_SIZE;
        let segment = FrameAllocOptions::new()
            .alloc_segment(range.len() / PAGE_SIZE)
            .expect("Failed to allocate segment");
        assert_eq!(segment.size(), range.len());
        assert_eq!(segment.end_paddr() - segment.paddr(), range.len());
    }

    #[ktest]
    #[should_panic]
    fn max_segment_creation() {
        // Upstream FrameAllocator panics when attempting to allocate a segment with usize::MAX frames
        let max_frames = usize::MAX;
        let _ = FrameAllocOptions::new().alloc_segment(max_frames);
    }

    #[ktest]
    fn empty_segment_creation() {
        let result = FrameAllocOptions::new().alloc_segment(0);
        assert!(
            matches!(result, Err(Error::InvalidArgs)),
            "Expected `InvalidArgs` error when allocating a zero-sized segment"
        );
    }

    #[ktest]
    fn zeroed_segment_creation() {
        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(1)
            .expect("Failed to allocate segment");
        let mut reader = segment.reader();
        let mut buffer = [0; PAGE_SIZE];
        reader.read(&mut buffer.as_mut_slice().into());
        assert!(buffer.iter().all(|&x| x == 0));
    }

    #[ktest]
    fn segment_split() {
        let total_frames = 2;
        let segment = FrameAllocOptions::new()
            .alloc_segment(total_frames)
            .expect("Failed to allocate segment");
        let (first, second) = segment.split(PAGE_SIZE);
        assert_eq!(first.size(), PAGE_SIZE);
        assert_eq!(second.size(), PAGE_SIZE);
    }

    #[ktest]
    fn segment_slice() {
        let total_frames = 3;
        let segment = FrameAllocOptions::new()
            .alloc_segment(total_frames)
            .expect("Failed to allocate segment");
        let slice = segment.slice(&(PAGE_SIZE..PAGE_SIZE * 2));
        assert_eq!(slice.size(), PAGE_SIZE);
        assert_eq!(slice.paddr(), segment.paddr() + PAGE_SIZE);
    }

    #[ktest]
    fn segment_iteration() {
        let total_frames = 2;
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(total_frames, |_| MockFrameMeta { value: 42 })
            .expect("Failed to allocate segment");
        let mut count = 0;
        for frame in segment {
            assert_eq!(frame.meta().value, 42);
            count += 1;
        }
        assert_eq!(count, total_frames);
    }

    #[ktest]
    #[should_panic(expected = "segment virtual address out-of-bound for splitting")]
    fn invalid_segment_split() {
        let total_frames = 2;
        let segment = FrameAllocOptions::new()
            .alloc_segment(total_frames)
            .expect("Failed to allocate segment");
        // Attempts to split at zero, which should panic
        segment.split(0);
    }

    #[ktest]
    #[should_panic(expected = "segment virtual address out-of-bound for slicing")]
    fn segment_slice_out_of_bounds() {
        let total_frames = 3;
        let segment = FrameAllocOptions::new()
            .alloc_segment(total_frames)
            .expect("Failed to allocate segment");
        let huge_page_aligned_offset = usize::MAX - (PAGE_SIZE - 1);
        // Huge out-of-bounds offsets should panic
        segment.slice(&(huge_page_aligned_offset..huge_page_aligned_offset));
    }

    #[ktest]
    #[should_panic(expected = "segment virtual address empty for slicing")]
    fn segment_slice_empty() {
        let segment = FrameAllocOptions::new()
            .alloc_segment(1)
            .expect("Failed to allocate segment");
        // Slicing an empty range should panic
        segment.slice(&(0..0));
    }

    #[ktest]
    fn segment_iter_partial_drop() {
        let options = FrameAllocOptions::new();
        let segment = options
            .alloc_segment_with(2, |_| MockFrameMeta { value: 42 })
            .expect("Failed to allocate segment");

        let second = segment.slice(&(PAGE_SIZE..2 * PAGE_SIZE));

        let mut iter = segment.into_iter();
        let first = iter.next().expect("The segment is non-empty");

        // Dropping the iterator must release the frame it never yielded
        drop(iter);

        assert_eq!(first.reference_count(), 1);
        let second = second.into_iter().next().unwrap();
        assert_eq!(second.reference_count(), 1);
        assert_eq!(second.meta().value, 42);
    }

    #[ktest]
    fn segment_to_usegment() {
        let options = FrameAllocOptions::new();
        let segment = options.alloc_segment(1).unwrap();
        let dyn_segment: Segment<dyn AnyFrameMeta> = segment.clone().into();
        let result: Result<USegment, Segment<_>> = USegment::try_from(dyn_segment);
        assert!(result.is_ok());
        let usegment = result.unwrap();
        assert_eq!(usegment.size(), PAGE_SIZE);
        assert_eq!(usegment.paddr(), segment.paddr());
    }

    #[ktest]
    fn segment_to_segment() {
        let options = FrameAllocOptions::new();
        let segment = options
            .alloc_segment_with(1, |_| MockFrameMeta { value: 42 })
            .unwrap();
        let dyn_segment: Segment<dyn AnyFrameMeta> = segment.into();
        let result: Result<Segment<MockFrameMeta>, Segment<_>> = Segment::try_from(dyn_segment);
        assert!(result.is_ok());
        let segment = result.unwrap();
        assert_eq!(segment.size(), PAGE_SIZE);
        for frame in segment {
            assert_eq!(frame.meta().value, 42);
        }
    }

    #[ktest]
    fn frame_to_segment() {
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(MockFrameMeta { value: 42 })
            .unwrap();
        let paddr = frame.paddr();
        let segment: Segment<MockFrameMeta> = frame.into();
        assert_eq!(segment.size(), PAGE_SIZE);
        assert_eq!(segment.paddr(), paddr);
        for frame in segment {
            assert_eq!(frame.meta().value, 42);
        }
    }

    #[ktest]
    fn segment_drop() {
        let options = FrameAllocOptions::new();
        let segment = options.alloc_segment(1).unwrap();
        let paddr_before = segment.paddr();
        drop(segment);
        let new_segment = options.alloc_segment(1).unwrap();
        assert_eq!(new_segment.paddr(), paddr_before);
    }
}

// Untyped frame/segment tests
mod untyped {
    use super::*;

    #[ktest]
    fn untyped_frame_reader_writer() {
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(())
            .expect("Failed to allocate frame");

        // Tests the frame reader
        let mut reader = frame.reader();
        assert_eq!(reader.remain(), PAGE_SIZE);

        // Tests the frame writer
        let mut writer = frame.writer();
        assert_eq!(writer.avail(), PAGE_SIZE);

        // Writes data to the frame
        let data = [0xAA; 128];
        writer.write(&mut data.as_slice().into());

        // Reads data back
        let mut buffer = [0; 128];
        reader.read(&mut buffer.as_mut_slice().into());
        assert_eq!(buffer, data);
    }

    #[ktest]
    fn untyped_segment_reader_writer() {
        let segment = FrameAllocOptions::new()
            .alloc_segment(2)
            .expect("Failed to allocate segment");

        // Tests the segment reader
        let mut reader = segment.reader();
        assert_eq!(reader.remain(), 2 * PAGE_SIZE);

        // Tests the segment writer
        let mut writer = segment.writer();
        assert_eq!(writer.avail(), 2 * PAGE_SIZE);

        // Writes data to the segment
        let data = [0xBB; 256];
        writer.write(&mut data.as_slice().into());

        // Reads data back
        let mut buffer = [0; 256];
        reader.read(&mut buffer.as_mut_slice().into());
        assert_eq!(buffer, data);
    }
}

mod frame_ref {
    use super::*;
    use crate::sync::non_null::NonNullPtr;

    #[ktest]
    fn frame_ref_preserves_refcnt() {
        let init_val = 42;
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(MockUFrameMeta { value: init_val })
            .expect("Failed to allocate frame");

        assert_eq!(frame.reference_count(), 1);

        {
            let frame_ref = frame.borrow();
            assert_eq!(frame_ref.meta().value, init_val);
            assert_eq!(frame_ref.reference_count(), 1);
            assert_eq!(frame.reference_count(), 1);
        }

        assert_eq!(frame.reference_count(), 1);
    }

    #[ktest]
    fn frame_impls_non_null_ptr() {
        let init_val = 42;
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(MockUFrameMeta { value: init_val })
            .expect("Failed to allocate frame");
        let ptr = frame.paddr();
        let uframe: UFrame = frame.into();

        // Converts and retrieves the frame from raw pointer
        let raw_ptr = NonNullPtr::into_raw(uframe);
        let frame_from_raw: Frame<MockUFrameMeta> = unsafe { NonNullPtr::from_raw(raw_ptr.cast()) };
        assert_eq!(frame_from_raw.paddr(), ptr);
        assert_eq!(frame_from_raw.meta().value, init_val);

        // References the frame from raw pointer
        let frame_ref: FrameRef<MockUFrameMeta> = unsafe { Frame::raw_as_ref(raw_ptr.cast()) };
        assert_eq!(frame_ref.paddr(), ptr);
    }
}

// Address-sensitive frame metadata tests.
mod pinning {
    use core::{marker::PhantomPinned, pin::Pin};

    use super::*;
    use crate::mm::{
        Infallible, UniqueFrame, VmReader,
        frame::linked_list::{Link, LinkedList},
    };

    #[derive(Default)]
    struct DropCounts {
        callbacks: AtomicUsize,
        drops: AtomicUsize,
    }

    #[derive(Default)]
    struct PinnedMeta {
        address: AtomicUsize,
        counts: Arc<DropCounts>,
        _pin: PhantomPinned,
    }

    impl PinnedMeta {
        fn record_address(self: Pin<&mut Self>) {
            let address = self.as_ref().get_ref() as *const Self as usize;
            self.address.store(address, Ordering::Relaxed);
        }

        fn check_address(&self) {
            assert_eq!(
                self as *const Self as usize,
                self.address.load(Ordering::Relaxed)
            );
        }
    }

    // SAFETY: The callback does not read bytes. Address dependencies are only
    // initialized after pinning, and the metadata is !Unpin and never moved.
    unsafe impl AnyFrameMeta for PinnedMeta {
        unsafe fn on_drop(self: Pin<&mut Self>, _reader: &mut VmReader<Infallible>) {
            self.check_address();
            assert_eq!(self.counts.callbacks.fetch_add(1, Ordering::Relaxed), 0);
        }
    }

    impl Drop for PinnedMeta {
        fn drop(&mut self) {
            if self.address.load(Ordering::Relaxed) != 0 {
                self.check_address();
            }
            self.counts.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn new_frame() -> (UniqueFrame<Link<PinnedMeta>>, Arc<DropCounts>) {
        let meta = PinnedMeta::default();
        let counts = meta.counts.clone();
        let mut frame: UniqueFrame<_> = FrameAllocOptions::new()
            .alloc_frame_with(Link::new(meta))
            .unwrap()
            .try_into()
            .unwrap();
        frame.pin_meta().project().record_address();
        (frame, counts)
    }

    #[ktest]
    fn pinned_metadata_survives_list_operations() {
        let (first, first_counts) = new_frame();
        let (second, second_counts) = new_frame();
        let first_paddr = first.paddr();
        let mut list = LinkedList::new();
        list.push_back(first);
        list.push_front(second);

        let mut cursor = list.cursor_mut_at(first_paddr).unwrap();
        cursor.pin_current_meta().unwrap().check_address();
        let first = cursor.take_current().unwrap();
        assert!(cursor.pin_current_meta().is_none());
        first.meta().check_address();
        list.push_front(first);

        drop(list);
        for counts in [first_counts, second_counts] {
            assert_eq!(counts.callbacks.load(Ordering::Relaxed), 1);
            assert_eq!(counts.drops.load(Ordering::Relaxed), 1);
        }
    }

    #[ktest]
    fn pinned_metadata_survives_frame_conversions() {
        let (frame, counts) = new_frame();
        let paddr = frame.paddr();
        let shared: Frame<_> = frame.into();
        let other = shared.clone();
        drop(other);

        let erased: Frame<dyn AnyFrameMeta> = shared.into();
        let unique: UniqueFrame<dyn AnyFrameMeta> = erased.try_into().unwrap();
        assert_eq!(unique.paddr(), paddr);
        assert_eq!(
            unique.dyn_meta().type_id(),
            core::any::TypeId::of::<Link<PinnedMeta>>()
        );

        let shared: Frame<dyn AnyFrameMeta> = unique.into();
        let typed = Frame::<Link<PinnedMeta>>::try_from(shared).unwrap();
        let mut unique: UniqueFrame<_> = typed.try_into().unwrap();
        unique.pin_meta().project().check_address();

        drop(unique.repurpose(()));
        assert_eq!(counts.callbacks.load(Ordering::Relaxed), 1);
        assert_eq!(counts.drops.load(Ordering::Relaxed), 1);
    }

    #[ktest]
    fn pinned_replacement_runs_ordinary_drop() {
        let (mut frame, old_counts) = new_frame();
        let replacement = PinnedMeta::default();
        let new_counts = replacement.counts.clone();
        frame.pin_meta().project().set(replacement);
        assert_eq!(old_counts.callbacks.load(Ordering::Relaxed), 0);
        assert_eq!(old_counts.drops.load(Ordering::Relaxed), 1);

        let paddr = frame.paddr();
        frame.pin_meta().project().record_address();
        frame.reset_as_unused();
        assert_eq!(new_counts.callbacks.load(Ordering::Relaxed), 1);
        assert_eq!(new_counts.drops.load(Ordering::Relaxed), 1);
        drop(UniqueFrame::from_unused(paddr, ()).unwrap());
    }
}

mod teardown {
    use core::pin::Pin;

    use super::*;
    use crate::{
        mm::{self, Infallible, UniqueFrame, VmReader},
        panic,
    };

    const INSTALLED: usize = 0;
    const TORN_DOWN: usize = 1;
    const DROPPED: usize = 2;

    struct TeardownMeta {
        stage: Arc<AtomicUsize>,
        paddr: Paddr,
        should_panic_on_teardown: bool,
    }

    // SAFETY: The callback only checks the reader's bounds without reading bytes.
    unsafe impl AnyFrameMeta for TeardownMeta {
        unsafe fn on_drop(self: Pin<&mut Self>, reader: &mut VmReader<Infallible>) {
            assert_eq!(reader.cursor() as usize, mm::paddr_to_vaddr(self.paddr));
            assert_eq!(reader.remain(), PAGE_SIZE);
            assert_eq!(self.stage.swap(TORN_DOWN, Ordering::Relaxed), INSTALLED);
            assert!(!self.should_panic_on_teardown, "teardown panic");
        }
    }

    impl Drop for TeardownMeta {
        fn drop(&mut self) {
            assert_eq!(self.stage.swap(DROPPED, Ordering::Relaxed), TORN_DOWN);
        }
    }

    fn new_frame() -> (UniqueFrame<TeardownMeta>, Arc<AtomicUsize>) {
        let stage = Arc::new(AtomicUsize::new(INSTALLED));
        let mut frame: UniqueFrame<_> = FrameAllocOptions::new()
            .alloc_frame_with(TeardownMeta {
                stage: stage.clone(),
                paddr: 0,
                should_panic_on_teardown: false,
            })
            .unwrap()
            .try_into()
            .unwrap();
        frame.meta_mut().paddr = frame.paddr();
        (frame, stage)
    }

    #[ktest]
    fn release_calls_teardown_once_before_drop() {
        let (frame, stage) = new_frame();
        drop(frame);
        assert_eq!(stage.load(Ordering::Relaxed), DROPPED);

        let (frame, stage) = new_frame();
        let frame = Frame::from(frame);
        let other = frame.clone();
        drop(frame);
        assert_eq!(stage.load(Ordering::Relaxed), INSTALLED);
        drop(other);
        assert_eq!(stage.load(Ordering::Relaxed), DROPPED);
    }

    #[ktest]
    fn repurpose_calls_teardown_once_before_drop() {
        let (frame, stage) = new_frame();
        let paddr = frame.paddr();
        let frame = frame.repurpose(MockFrameMeta { value: 42 });
        assert_eq!(stage.load(Ordering::Relaxed), DROPPED);
        assert_eq!(frame.paddr(), paddr);
        assert_eq!(frame.meta().value, 42);
    }

    #[ktest]
    fn reset_calls_teardown_once_before_drop() {
        let (frame, stage) = new_frame();
        let paddr = frame.paddr();
        frame.reset_as_unused();
        assert_eq!(stage.load(Ordering::Relaxed), DROPPED);
        // Reset does not return the page to the allocator. Reclaim it to clean up.
        drop(UniqueFrame::from_unused(paddr, ()).unwrap());
    }

    #[ktest]
    fn panicking_teardown_retains_frame() {
        fn assert_retained(teardown_fn: impl FnOnce(UniqueFrame<TeardownMeta>)) {
            let (mut frame, stage) = new_frame();
            frame.meta_mut().should_panic_on_teardown = true;
            let paddr = frame.paddr();
            assert!(panic::catch_unwind(|| teardown_fn(frame)).is_err());
            assert_eq!(stage.load(Ordering::Relaxed), TORN_DOWN);
            assert!(UniqueFrame::from_unused(paddr, ()).is_err());
            assert!(Frame::<dyn AnyFrameMeta>::from_in_use(paddr).is_err());
            // The failed teardown must leave the frame allocated. Intentionally
            // leak it: accessing its metadata or retrying teardown is unsound.
        }

        assert_retained(drop);
        assert_retained(|frame| drop(Frame::from(frame)));
        assert_retained(|frame| drop(frame.repurpose(())));
        assert_retained(UniqueFrame::reset_as_unused);
    }

    #[ktest]
    fn custom_teardown_runs_once() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);

        struct CustomDropMeta;
        impl_untyped_frame_meta_for!(CustomDropMeta, {
            CALLS.fetch_add(1, Ordering::Relaxed);
        });

        let before = CALLS.load(Ordering::Relaxed);
        let frame = FrameAllocOptions::new()
            .alloc_frame_with(CustomDropMeta)
            .unwrap();
        let other = frame.clone();
        drop(frame);
        assert_eq!(CALLS.load(Ordering::Relaxed), before);
        drop(other);
        assert_eq!(CALLS.load(Ordering::Relaxed), before + 1);
    }
}
