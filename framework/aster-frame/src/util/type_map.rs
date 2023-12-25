/// A type map is a collection whose keys are types, rather than values.
pub struct TypeMap {}

pub trait Any: core::any::Any + Send + Sync {}

impl TypeMap {
    /// Creates an empty typed map.
    pub fn new() -> Self {
        todo!()
    }

    /// Inserts a new item of type `T`.
    pub fn insert<T: Any>(&mut self, val: T) -> Option<T> {
        todo!()
    }

    /// Gets an item of type `T`.
    pub fn get<T: Any>(&self) -> Option<&T> {
        todo!()
    }

    /// Gets an item of type `T`.
    pub fn remove<T: Any>(&self) -> Option<T> {
        todo!()
    }
}
