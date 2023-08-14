pub type Result<T> = core::result::Result<T, self::Error>;

/// Errors
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    BadMagic,
    BadRevision,
    BadCreaterOS,
    BadBitMap,
    BadDirEntry,
    NotSupported,
    IsDir,
    NotDir,
    NotFound,
    Exist,
    NotSameFs,
    InvalidParam,
    NoSpace,
    DirRemoved,
    DirNotEmpty,
    Again,
    SymLoop,
    Busy,
    IoError,
    WriteProtected,
    NoPerm,
    NameTooLong,
    FileTooBig,
    OpNotSupported,
    NoMemory,
}

impl From<mem_storage::Error> for Error {
    fn from(mem_error: mem_storage::Error) -> Self {
        match mem_error {
            mem_storage::Error::AccessDenied => Error::NoPerm,
            mem_storage::Error::NoMemory => Error::NoMemory,
            mem_storage::Error::InvalidArgs => Error::InvalidParam,
            mem_storage::Error::IoError => Error::IoError,
            mem_storage::Error::NotEnoughResources => Error::Busy,
            mem_storage::Error::InvalidVmpermBits => Error::InvalidParam,
            mem_storage::Error::PageFault => Error::IoError,
            mem_storage::Error::NoChild => Error::IoError,
        }
    }
}
