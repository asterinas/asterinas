use crate::task::Priority;

/// The Nice value of a task.
///
/// # Range
///
/// ```
/// assert!(Nice::MIN <= 0 && Nice::MAX >= 0);
///
/// fn prio_num_is_in_i8() -> bool {
///     let lower_bound = Priority::highest().get();
///     let upper_bound = Priority::lowest().get();
///     lower_bound as i64 >= i8::MIN.into() && upper_bound as u64 <= i8::MAX as u64
/// }
/// assert!(prio_num_is_in_i8());
/// ```
pub type Nice = i8;

/// The exclusive upper bound of the numeric value of a real-time priority.
///
/// ```
/// assert_eq!(RT_PRIO_NUMERIC_UPPER_BOUND, 100);
/// ```
const RT_PRIO_NUMERIC_UPPER_BOUND: u16 = Priority::normal().get();

impl From<Priority> for Nice {
    /// Convert static priority [ MAX_RT_PRIO..MAX_PRIO ]
    /// to user-nice values [ -20 ... 0 ... 19 ]
    fn from(prio: Priority) -> Self {
        (prio.get() as i8 - RT_PRIO_NUMERIC_UPPER_BOUND as i8) as Nice - 20
    }
}
