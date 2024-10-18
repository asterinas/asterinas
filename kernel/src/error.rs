// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

/// Error number.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Errno {
    EPERM = 1,    /* Operation not permitted */
    ENOENT = 2,   /* No such file or directory */
    ESRCH = 3,    /* No such process */
    EINTR = 4,    /* Interrupted system call */
    EIO = 5,      /* I/O error */
    ENXIO = 6,    /* No such device or address */
    E2BIG = 7,    /* Argument list too long */
    ENOEXEC = 8,  /* Exec format error */
    EBADF = 9,    /* Bad file number */
    ECHILD = 10,  /* No child processes */
    EAGAIN = 11,  /* Try again */
    ENOMEM = 12,  /* Out of memory */
    EACCES = 13,  /* Permission denied */
    EFAULT = 14,  /* Bad address */
    ENOTBLK = 15, /* Block device required */
    EBUSY = 16,   /* Device or resource busy */
    EEXIST = 17,  /* File exists */
    EXDEV = 18,   /* Cross-device link */
    ENODEV = 19,  /* No such device */
    ENOTDIR = 20, /* Not a directory */
    EISDIR = 21,  /* Is a directory */
    EINVAL = 22,  /* Invalid argument */
    ENFILE = 23,  /* File table overflow */
    EMFILE = 24,  /* Too many open files */
    ENOTTY = 25,  /* Not a typewriter */
    ETXTBSY = 26, /* Text file busy */
    EFBIG = 27,   /* File too large */
    ENOSPC = 28,  /* No space left on device */
    ESPIPE = 29,  /* Illegal seek */
    EROFS = 30,   /* Read-only file system */
    EMLINK = 31,  /* Too many links */
    EPIPE = 32,   /* Broken pipe */
    EDOM = 33,    /* Math argument out of domain of func */
    ERANGE = 34,  /* Math result not representable */

    EDEADLK = 35,      /* Resource deadlock would occur */
    ENAMETOOLONG = 36, /* File name too long */
    ENOLCK = 37,       /* No record locks available */
    /*
     * This error code is special: arch syscall entry code will return
     * -ENOSYS if users try to call a syscall that doesn't exist.  To keep
     * failures of syscalls that really do exist distinguishable from
     * failures due to attempts to use a nonexistent syscall, syscall
     * implementations should refrain from returning -ENOSYS.
     */
    ENOSYS = 38,    /* Invalid system call number */
    ENOTEMPTY = 39, /* Directory not empty */
    ELOOP = 40,     /* Too many symbolic links encountered */
    // EWOULDBLOCK	EAGAIN	/* Operation would block */
    ENOMSG = 42,   /* No message of desired type */
    EIDRM = 43,    /* Identifier removed */
    ECHRNG = 44,   /* Channel number out of range */
    EL2NSYNC = 45, /* Level 2 not synchronized */
    EL3HLT = 46,   /* Level 3 halted */
    EL3RST = 47,   /* Level 3 reset */
    ELNRNG = 48,   /* Link number out of range */
    EUNATCH = 49,  /* Protocol driver not attached */
    ENOCSI = 50,   /* No CSI structure available */
    EL2HLT = 51,   /* Level 2 halted */
    EBADE = 52,    /* Invalid exchange */
    EBADR = 53,    /* Invalid request descriptor */
    EXFULL = 54,   /* Exchange full */
    ENOANO = 55,   /* No anode */
    EBADRQC = 56,  /* Invalid request code */
    EBADSLT = 57,  /* Invalid slot */
    // EDEADLOCK	EDEADLK
    EBFONT = 59,          /* Bad font file format */
    ENOSTR = 60,          /* Device not a stream */
    ENODATA = 61,         /* No data available */
    ETIME = 62,           /* Timer expired */
    ENOSR = 63,           /* Out of streams resources */
    ENONET = 64,          /* Machine is not on the network */
    ENOPKG = 65,          /* Package not installed */
    EREMOTE = 66,         /* Object is remote */
    ENOLINK = 67,         /* Link has been severed */
    EADV = 68,            /* Advertise error */
    ESRMNT = 69,          /* Srmount error */
    ECOMM = 70,           /* Communication error on send */
    EPROTO = 71,          /* Protocol error */
    EMULTIHOP = 72,       /* Multihop attempted */
    EDOTDOT = 73,         /* RFS specific error */
    EBADMSG = 74,         /* Not a data message */
    EOVERFLOW = 75,       /* Value too large for defined data type */
    ENOTUNIQ = 76,        /* Name not unique on network */
    EBADFD = 77,          /* File descriptor in bad state */
    EREMCHG = 78,         /* Remote address changed */
    ELIBACC = 79,         /* Can not access a needed shared library */
    ELIBBAD = 80,         /* Accessing a corrupted shared library */
    ELIBSCN = 81,         /* .lib section in a.out corrupted */
    ELIBMAX = 82,         /* Attempting to link in too many shared libraries */
    ELIBEXEC = 83,        /* Cannot exec a shared library directly */
    EILSEQ = 84,          /* Illegal byte sequence */
    ERESTART = 85,        /* Interrupted system call should be restarted */
    ESTRPIPE = 86,        /* Streams pipe error */
    EUSERS = 87,          /* Too many users */
    ENOTSOCK = 88,        /* Socket operation on non-socket */
    EDESTADDRREQ = 89,    /* Destination address required */
    EMSGSIZE = 90,        /* Message too long */
    EPROTOTYPE = 91,      /* Protocol wrong type for socket */
    ENOPROTOOPT = 92,     /* Protocol not available */
    EPROTONOSUPPORT = 93, /* Protocol not supported */
    ESOCKTNOSUPPORT = 94, /* Socket type not supported */
    EOPNOTSUPP = 95,      /* Operation not supported on transport endpoint */
    EPFNOSUPPORT = 96,    /* Protocol family not supported */
    EAFNOSUPPORT = 97,    /* Address family not supported by protocol */
    EADDRINUSE = 98,      /* Address already in use */
    EADDRNOTAVAIL = 99,   /* Cannot assign requested address */
    ENETDOWN = 100,       /* Network is down */
    ENETUNREACH = 101,    /* Network is unreachable */
    ENETRESET = 102,      /* Network dropped connection because of reset */
    ECONNABORTED = 103,   /* Software caused connection abort */
    ECONNRESET = 104,     /* Connection reset by peer */
    ENOBUFS = 105,        /* No buffer space available */
    EISCONN = 106,        /* Transport endpoint is already connected */
    ENOTCONN = 107,       /* Transport endpoint is not connected */
    ESHUTDOWN = 108,      /* Cannot send after transport endpoint shutdown */
    ETOOMANYREFS = 109,   /* Too many references: cannot splice */
    ETIMEDOUT = 110,      /* Connection timed out */
    ECONNREFUSED = 111,   /* Connection refused */
    EHOSTDOWN = 112,      /* Host is down */
    EHOSTUNREACH = 113,   /* No route to host */
    EALREADY = 114,       /* Operation already in progress */
    EINPROGRESS = 115,    /* Operation now in progress */
    ESTALE = 116,         /* Stale file handle */
    EUCLEAN = 117,        /* Structure needs cleaning */
    ENOTNAM = 118,        /* Not a XENIX named type file */
    ENAVAIL = 119,        /* No XENIX semaphores available */
    EISNAM = 120,         /* Is a named type file */
    EREMOTEIO = 121,      /* Remote I/O error */
    EDQUOT = 122,         /* Quota exceeded */
    ENOMEDIUM = 123,      /* No medium found */
    EMEDIUMTYPE = 124,    /* Wrong medium type */
    ECANCELED = 125,      /* Operation Canceled */
    ENOKEY = 126,         /* Required key not available */
    EKEYEXPIRED = 127,    /* Key has expired */
    EKEYREVOKED = 128,    /* Key has been revoked */
    EKEYREJECTED = 129,   /* Key was rejected by service */
    /* for robust mutexes */
    EOWNERDEAD = 130,      /* Owner died */
    ENOTRECOVERABLE = 131, /* State not recoverable */

    ERFKILL = 132, /* Operation not possible due to RF-kill */

    EHWPOISON = 133, /* Memory page has hardware error */
}

/// error used in this crate
#[derive(Debug, Clone, Copy)]
pub struct Error {
    errno: Errno,
    msg: Option<&'static str>,
}

impl Error {
    pub const fn new(errno: Errno) -> Self {
        Error { errno, msg: None }
    }

    pub const fn with_message(errno: Errno, msg: &'static str) -> Self {
        Error {
            errno,
            msg: Some(msg),
        }
    }

    pub const fn error(&self) -> Errno {
        self.errno
    }
}

impl From<Errno> for Error {
    fn from(errno: Errno) -> Self {
        Error::new(errno)
    }
}

impl AsRef<Error> for Error {
    fn as_ref(&self) -> &Error {
        self
    }
}

impl From<ostd::Error> for Error {
    fn from(ostd_error: ostd::Error) -> Self {
        match ostd_error {
            ostd::Error::AccessDenied => Error::new(Errno::EFAULT),
            ostd::Error::NoMemory => Error::new(Errno::ENOMEM),
            ostd::Error::InvalidArgs => Error::new(Errno::EINVAL),
            ostd::Error::IoError => Error::new(Errno::EIO),
            ostd::Error::NotEnoughResources => Error::new(Errno::EBUSY),
            ostd::Error::PageFault => Error::new(Errno::EFAULT),
            ostd::Error::Overflow => Error::new(Errno::EOVERFLOW),
            ostd::Error::MapAlreadyMappedVaddr => Error::new(Errno::EINVAL),
            ostd::Error::KVirtAreaAllocError => Error::new(Errno::ENOMEM),
        }
    }
}

impl From<(ostd::Error, usize)> for Error {
    // Used in fallible memory read/write API
    fn from(ostd_error: (ostd::Error, usize)) -> Self {
        Error::from(ostd_error.0)
    }
}

impl From<aster_block::bio::BioEnqueueError> for Error {
    fn from(error: aster_block::bio::BioEnqueueError) -> Self {
        match error {
            aster_block::bio::BioEnqueueError::IsFull => {
                Error::with_message(Errno::EBUSY, "The request queue is full")
            }
            aster_block::bio::BioEnqueueError::Refused => {
                Error::with_message(Errno::EBUSY, "Refuse to enqueue the bio")
            }
            aster_block::bio::BioEnqueueError::TooBig => {
                Error::with_message(Errno::EINVAL, "Bio is too big")
            }
        }
    }
}

impl From<aster_block::bio::BioStatus> for Error {
    fn from(err_status: aster_block::bio::BioStatus) -> Self {
        match err_status {
            aster_block::bio::BioStatus::NotSupported => {
                Error::with_message(Errno::EIO, "I/O operation is not supported")
            }
            aster_block::bio::BioStatus::NoSpace => {
                Error::with_message(Errno::ENOSPC, "Insufficient space on device")
            }
            aster_block::bio::BioStatus::IoError => {
                Error::with_message(Errno::EIO, "I/O operation fails")
            }
            status => panic!("Can not convert the status: {:?} to an error", status),
        }
    }
}

impl From<core::str::Utf8Error> for Error {
    fn from(_: core::str::Utf8Error) -> Self {
        Error::with_message(Errno::EINVAL, "Invalid utf-8 string")
    }
}

impl From<alloc::string::FromUtf8Error> for Error {
    fn from(_: alloc::string::FromUtf8Error) -> Self {
        Error::with_message(Errno::EINVAL, "Invalid utf-8 string")
    }
}

impl From<core::ffi::FromBytesUntilNulError> for Error {
    fn from(_: core::ffi::FromBytesUntilNulError) -> Self {
        Error::with_message(Errno::E2BIG, "Cannot find null in cstring")
    }
}

impl From<core::ffi::FromBytesWithNulError> for Error {
    fn from(_: core::ffi::FromBytesWithNulError) -> Self {
        Error::with_message(Errno::E2BIG, "Cannot find null in cstring")
    }
}

impl From<cpio_decoder::error::Error> for Error {
    fn from(cpio_error: cpio_decoder::error::Error) -> Self {
        match cpio_error {
            cpio_decoder::error::Error::MagicError => {
                Error::with_message(Errno::EINVAL, "CPIO invalid magic number")
            }
            cpio_decoder::error::Error::Utf8Error => {
                Error::with_message(Errno::EINVAL, "CPIO invalid utf-8 string")
            }
            cpio_decoder::error::Error::ParseIntError => {
                Error::with_message(Errno::EINVAL, "CPIO parse int error")
            }
            cpio_decoder::error::Error::FileTypeError => {
                Error::with_message(Errno::EINVAL, "CPIO invalid file type")
            }
            cpio_decoder::error::Error::FileNameError => {
                Error::with_message(Errno::EINVAL, "CPIO invalid file name")
            }
            cpio_decoder::error::Error::BufferShortError => {
                Error::with_message(Errno::EINVAL, "CPIO buffer is too short")
            }
            cpio_decoder::error::Error::IoError => {
                Error::with_message(Errno::EIO, "CPIO buffer I/O error")
            }
        }
    }
}

impl From<Error> for ostd::Error {
    fn from(error: Error) -> Self {
        match error.errno {
            Errno::EACCES => ostd::Error::AccessDenied,
            Errno::EIO => ostd::Error::IoError,
            Errno::ENOMEM => ostd::Error::NoMemory,
            Errno::EFAULT => ostd::Error::PageFault,
            Errno::EINVAL => ostd::Error::InvalidArgs,
            Errno::EBUSY => ostd::Error::NotEnoughResources,
            _ => ostd::Error::InvalidArgs,
        }
    }
}

impl From<alloc::ffi::NulError> for Error {
    fn from(_: alloc::ffi::NulError) -> Self {
        Error::with_message(Errno::E2BIG, "Cannot find null in cstring")
    }
}

impl From<int_to_c_enum::TryFromIntError> for Error {
    fn from(_: int_to_c_enum::TryFromIntError) -> Self {
        Error::with_message(Errno::EINVAL, "Invalid enum value")
    }
}

#[macro_export]
macro_rules! return_errno {
    ($errno: expr) => {
        return Err($crate::error::Error::new($errno))
    };
}

#[macro_export]
macro_rules! return_errno_with_message {
    ($errno: expr, $message: expr) => {
        return Err($crate::error::Error::with_message($errno, $message))
    };
}
