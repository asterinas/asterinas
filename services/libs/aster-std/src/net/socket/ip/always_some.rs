use crate::prelude::*;
use core::ops::{Deref, DerefMut};

/// AlwaysSome is a wrapper for Option.
///
/// AlwaysSome should always be Some(T), so we can treat it as a smart pointer.
/// If it becomes None, the AlwaysSome should be viewed invalid and cannot be used anymore.
pub struct AlwaysSome<T>(Option<T>);

impl<T> AlwaysSome<T> {
    pub fn new(value: T) -> Self {
        AlwaysSome(Some(value))
    }

    pub fn try_take_with<R, E: Into<Error>, F: FnOnce(T) -> core::result::Result<R, (E, T)>>(
        &mut self,
        f: F,
    ) -> Result<R> {
        let value = if let Some(value) = self.0.take() {
            value
        } else {
            return_errno_with_message!(Errno::EINVAL, "the take cell is none");
        };
        match f(value) {
            Ok(res) => Ok(res),
            Err((err, t)) => {
                self.0 = Some(t);
                Err(err.into())
            }
        }
    }

    /// Takes inner value
    pub fn take(&mut self) -> T {
        debug_assert!(self.0.is_some());
        self.0.take().unwrap()
    }
}

impl<T> Deref for AlwaysSome<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}

impl<T> DerefMut for AlwaysSome<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap()
    }
}
