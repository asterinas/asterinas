use typeflags::typeflags;
use bitflags::bitflags;

bitflags! {
    /// Value-based access rights.
    /// 
    /// These access rights are provided to cover a wide range of use cases.
    /// The access rights' semantics and how they would restrict the behaviors 
    /// of a capability are decided by the capability's designer.
    /// Here, we give some sensible semantics for each access right.
    pub struct Rights: u32 {
        /// Allows duplicating a capability.
        const DUP: u32      = 1 << 0;
        /// Allows reading data from a data source (files, VM objects, etc.) or
        /// creating readable memory mappings.
        const READ: u32     = 1 << 1;
        /// Allows writing data to a data sink (files, VM objects, etc.) or
        /// creating writable memory mappings.
        const WRITE: u32    = 1 << 2;
        /// Allows creating executable memory mappings.
        const EXEC: u32     = 1 << 3;
        /// Allows sending notifications or signals.
        const SIGNAL: u32   = 1 << 7;
    }
}

typeflags! {
    /// Type-based access rights.
    /// 
    /// Similar to value-based access rights (`Rights`), but represented in
    /// types.
    pub trait TRights: u32 {
        /// Allows duplicating a capability.
        struct Dup: u32      = Rights::DUP;
        /// Allows reading data from a data source (files, VM objects, etc.) or
        /// creating readable memory mappings.
        struct Read: u32     = Rights::READ;
        /// Allows writing data to a data sink (files, VM objects, etc.) or
        /// creating writable memory mappings.
        struct Write: u32    = Rights::WRITE;
        /// Allows creating executable memory mappings.
        struct Exec: u32     = Rights::EXEC;
        /// Allows sending notifications or signals.
        struct Signal: u32   = Rights::SIGNAL;
    }
}

/// The full set of access rights.
pub type Full = TRights![
    Dup,
    Read,
    Write,
    Exec,
    Signal,
];
