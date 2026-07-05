use crate::abi::*;
use crate::util::*;

#[lang = "eh_personality"]
unsafe extern "C" fn personality(
    version: c_int,
    _actions: UnwindAction,
    _exception_class: u64,
    _exception: *mut UnwindException,
    _ctx: &mut UnwindContext<'_>,
) -> UnwindReasonCode {
    if version != 1 {
        return UnwindReasonCode::FATAL_PHASE1_ERROR;
    }
    UnwindReasonCode::CONTINUE_UNWIND
}
