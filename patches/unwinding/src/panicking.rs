use core::mem::ManuallyDrop;

use crate::abi::*;

pub unsafe trait Exception {
    const CLASS: [u8; 8];

    fn wrap(this: Self) -> *mut UnwindException;
    unsafe fn unwrap(ex: *mut UnwindException) -> Self;
}

pub fn begin_panic<E: Exception>(exception: E) -> UnwindReasonCode {
    unsafe extern "C" fn exception_cleanup<E: Exception>(
        _unwind_code: UnwindReasonCode,
        exception: *mut UnwindException,
    ) {
        unsafe { E::unwrap(exception) };
    }

    let ex = E::wrap(exception);
    unsafe {
        (*ex).exception_class = u64::from_ne_bytes(E::CLASS);
        (*ex).exception_cleanup = Some(exception_cleanup::<E>);
        _Unwind_RaiseException(ex)
    }
}

pub fn catch_unwind<E: Exception, R, F: FnOnce() -> R>(f: F) -> Result<R, Option<E>> {
    #[repr(C)]
    union Data<F, R, E> {
        f: ManuallyDrop<F>,
        r: ManuallyDrop<R>,
        p: ManuallyDrop<Option<E>>,
    }

    let mut data = Data {
        f: ManuallyDrop::new(f),
    };

    let data_ptr = &mut data as *mut _ as *mut u8;
    unsafe {
        return if core::intrinsics::catch_unwind(do_call::<F, R>, data_ptr, do_catch::<E>) != 0 {
            Err(ManuallyDrop::into_inner(data.p))
        } else {
            Ok(ManuallyDrop::into_inner(data.r))
        };
    }

    #[inline]
    fn do_call<F: FnOnce() -> R, R>(data: *mut u8) {
        unsafe {
            let data = &mut *(data as *mut Data<F, R, ()>);
            let f = ManuallyDrop::take(&mut data.f);
            data.r = ManuallyDrop::new(f());
        }
    }

    #[cold]
    fn do_catch<E: Exception>(data: *mut u8, exception: *mut u8) {
        unsafe {
            let data = &mut *(data as *mut ManuallyDrop<Option<E>>);
            let exception = exception as *mut UnwindException;
            if (*exception).exception_class != u64::from_ne_bytes(E::CLASS) {
                _Unwind_DeleteException(exception);
                *data = ManuallyDrop::new(None);
                return;
            }
            *data = ManuallyDrop::new(Some(E::unwrap(exception)));
        }
    }
}
