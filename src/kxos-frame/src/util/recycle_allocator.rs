use alloc::vec::Vec;

pub struct RecycleAllocator {
    current: usize,
    recycled: Vec<usize>,
    max: usize,
}

impl RecycleAllocator {
    pub fn new() -> Self {
        RecycleAllocator {
            current: 0,
            recycled: Vec::new(),
            max: usize::MAX - 1,
        }
    }

    pub fn with_start_max(start: usize, max: usize) -> Self {
        RecycleAllocator {
            current: start,
            recycled: Vec::new(),
            max: max,
        }
    }

    #[allow(unused)]
    pub fn alloc(&mut self) -> usize {
        if self.current == self.max && self.recycled.is_empty() {
            return usize::MAX;
        }
        if let Some(id) = self.recycled.pop() {
            id
        } else {
            self.current += 1;
            self.current - 1
        }
    }
    #[allow(unused)]
    pub fn dealloc(&mut self, id: usize) {
        assert!(id < self.current);
        assert!(
            !self.recycled.iter().any(|i| *i == id),
            "id {} has been deallocated!",
            id
        );
        self.recycled.push(id);
    }
}
