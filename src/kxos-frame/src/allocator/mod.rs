mod buddy_system_allocator;

pub fn init() {
    buddy_system_allocator::init_heap();
}
