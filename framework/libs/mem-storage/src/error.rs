/// The error type if copied from jinux-frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    InvalidArgs,
    NoMemory,
    PageFault,
    AccessDenied,
    IoError,
    InvalidVmpermBits,
    NotEnoughResources,
    NoChild,
}
