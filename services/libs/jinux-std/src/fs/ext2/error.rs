pub type Result<T> = core::result::Result<T, self::Error>;

/// Errors for Ext2
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

impl From<jinux_frame::Error> for Error {
    fn from(error: jinux_frame::Error) -> Self {
        match error {
            jinux_frame::Error::AccessDenied => Error::NoPerm,
            jinux_frame::Error::NoMemory => Error::NoMemory,
            jinux_frame::Error::InvalidArgs => Error::InvalidParam,
            jinux_frame::Error::IoError => Error::IoError,
            jinux_frame::Error::NotEnoughResources => Error::Busy,
            jinux_frame::Error::InvalidVmpermBits => Error::InvalidParam,
            jinux_frame::Error::PageFault => Error::IoError,
            jinux_frame::Error::NoChild => Error::IoError,
        }
    }
}
