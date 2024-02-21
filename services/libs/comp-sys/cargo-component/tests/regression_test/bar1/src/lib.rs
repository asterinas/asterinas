pub static BAR: &'static usize = &foo1::FOO_ITEM;

pub fn add(left: usize, right: usize) -> usize {
    foo1::foo_add(left, right)
}
