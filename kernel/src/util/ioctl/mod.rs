// SPDX-License-Identifier: MPL-2.0

//! Decoding and dispatching ioctl commands.
//!
//! When the system call layer handles an ioctl system call, it creates a [`RawIoctl`], which is
//! basically the numeric ioctl command and argument.
//!
//! The component that handles the specific ioctl logic should first convert the [`RawIoctl`] to a
//! strongly typed [`Ioctl`] instance, which decodes the ioctl command and includes type
//! information about the ioctl argument.
//!
//! To achieve this, define type aliases for [`Ioctl`] using the [`ioc`] macro, preferably in a
//! separate module for clarity. At the function that dispatches the ioctl commands, we can
//! glob-import the defined [`Ioctl`] type aliases and use the [`dispatch_ioctl`] macro for the
//! ioctl dispatching.
//!
//! Here is an complete example that demonstrates the basic usage:
//! ```
//! mod ioctl_defs {
//!     use crate::util::ioctl::{ioc, InData, OutData, PassByVal};
//!
//!     // Here we give the code in Linux to provide an intuitive guide on how to use the `ioc`
//!     // macro and how it corresponds to the Linux definitions. This is for demonstration
//!     // purposes only. For regular code, keeping the reference link below is sufficient.
//!     //
//!     // Also, note that the line containing the `ioc` macro will not be automatically formatted.
//!     // This is a deliberate design so that whitespaces can be manually inserted to vertically
//!     // align the `ioc` macro's parameters and improve code readability.
//!
//!     // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>>
//!
//!     // ```c
//!     // #define TIOCSCTTY    0x540E
//!     // ```
//!     pub(super) type SetControlTty = ioc!(TIOCSCTTY,  0x540E,     InData<i32, PassByVal>);
//!
//!     // ```c
//!     // #define TIOCSPTLCK   _IOW('T', 0x31, int)  /* Lock/unlock Pty */
//!     // ```
//!     pub(super) type SetPtyLock    = ioc!(TIOCSPTLCK, b'T', 0x31, InData<i32>);
//!
//!     // ```c
//!     // #define TIOCGPTLCK   _IOR('T', 0x39, int) /* Get Pty lock state */
//!     //  ```
//!     pub(super) type GetPtyLock    = ioc!(TIOCGPTLCK, b'T', 0x39, OutData<i32>);
//! }
//!
//! #[derive(Debug, Default)]
//! struct TtyState {
//!     is_controlling: bool,
//!     is_locked: bool,
//! }
//!
//! impl TtyState {
//!     fn ioctl(&mut self, raw_ioctl: RawIoctl) -> Result<()> {
//!         use ioctl_defs::*;
//!
//!         dispatch_ioctl!(match raw_ioctl {
//!             cmd @ SetControlTty => {
//!                 let _should_steal_tty = cmd.get() == 1;
//!                 self.is_controlling = true;
//!             }
//!             cmd @ SetPtyLock => {
//!                 self.is_locked = cmd.read()? != 0;
//!             }
//!             cmd @ GetPtyLock => {
//!                 cmd.write(if self.is_locked { &1 } else { &0 })?;
//!             }
//!             _ => return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown"),
//!         });
//!         Ok(())
//!     }
//! }
//!
//! fn fake_tty_ioctl() {
//!     let mut state = TtyState::default();
//!     assert!(!state.is_controlling);
//!
//!     state.ioctl(RawIoctl::new(0x540E, 0));
//!     assert!(state.is_controlling);
//! }
//! ```
//!
//! Additionally, we support the following advanced usage:
//!
//!  - For [`InOutData`], it is possible to obtain a [`SafePtr`] instance to the underlying data
//!    using [`Ioctl::with_data_ptr`]. This allows some structure fields to be treated as input and
//!    others as output. `TDX_CMD_GET_REPORT0` is such an example.
//!
//!  - For `OutData<[u8]>` (or `InData<[u8]>`), the argument size is encoded in the ioctl command.
//!    A [`VmWriter`] (or [`VmReader`]) can be obtained from [`Ioctl::with_writer`] (or
//!    [`Ioctl::with_reader`]), which allows for the writing (or reading) of variable-length data.
//!    `EVIOCGNAME` is such an example.
//!

use core::marker::PhantomData;

use aster_util::safe_ptr::SafePtr;
use sealed::{DataSpec, IoctlCmd, IoctlDir, PtrDataSpec};

use crate::{current_userspace, prelude::*};

mod sealed;

/// An ioctl command and its argument in raw form.
#[derive(Clone, Copy, Debug)]
pub struct RawIoctl {
    cmd: u32,
    arg: usize,
}

impl RawIoctl {
    /// Creates an instance with the given ioctl command and argument.
    pub const fn new(cmd: u32, arg: usize) -> Self {
        Self { cmd, arg }
    }

    /// Returns the ioctl command.
    pub const fn cmd(self) -> u32 {
        self.cmd
    }

    /// Returns the ioctl argument.
    pub const fn arg(self) -> usize {
        self.arg
    }
}

/// An ioctl command and its argument in strongly typed form.
///
/// `MAGIC` and `NR` are the Linux "magic" and "number" fields.
/// For legacy commands defined as raw constants, we may set them to the values
/// decoded from the raw number.
///
/// `IS_MODERN` indicates whether the ioctl is a modern or legacy one.
/// An legacy ioctl uses an arbitrary `u16` value as its number,
/// whereas a modern ioctl adopts a `u32` encoding.
///
/// `D` is one of [`NoData`], [`InData`], [`OutData`], or [`InOutData`].
/// It specifies key aspects about the input/output data in the ioctl argument.
pub struct Ioctl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, D> {
    cmd: IoctlCmd,
    arg: usize,
    _phantom: PhantomData<D>,
}

/// Defines an ioctl type.
///
/// # Legacy encoding
///
/// For legacy encoding, a 16-bit raw number is specified as the ioctl command.
///
/// For example,
/// ```
/// type NoTty = ioc!(TIOCNOTTY, 0x5422, NoData);
/// ```
/// It is equivalent to:
/// ```
/// type NoTty = Ioctl<0x54, 0x0E, /* IS_MODERN = */ false, NoData>;
/// ```
///
/// # Modern encoding
///
/// For modern encoding, a magic (type number) and a command ID (sequence number) are specified.
/// These, along with the data direction and size, compose the 32-bit ioctl command.
///
/// For example,
/// ```
/// type SetPtyLock = ioc!(TIOCSPTLCK, b'T', 0x31, InData<i32>);
/// ```
/// It is equivalent to:
/// ```
/// type SetPtyLock = Ioctl<b'T', 0x31, /* IS_MODERN = */ true, InData<i32>>;
/// ```
macro_rules! ioc {
    // Legacy encoding.
    ($linux_name:ident, $raw:literal, $data:ty) => {
        $crate::util::ioctl::Ioctl::<
            {
                // MAGIC
                $crate::util::ioctl::magic_and_nr_from_cmd($raw).0
            },
            {
                // NR
                $crate::util::ioctl::magic_and_nr_from_cmd($raw).1
            },
            {
                // IS_MODERN
                false
            },
            $data,
        >
    };
    // Modern encoding.
    ($linux_name:ident, $magic:literal, $nr:literal, $data:ty) => {
        $crate::util::ioctl::Ioctl::<{ $magic }, { $nr }, { true }, $data>
    };
}
pub(crate) use ioc;

/// Extracts the "magic" and "number" fields from an ioctl command.
#[doc(hidden)]
pub const fn magic_and_nr_from_cmd(raw_cmd: u16) -> (u8, u8) {
    let cmd = IoctlCmd::new(raw_cmd as u32);
    (cmd.magic(), cmd.nr())
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, D: DataSpec>
    Ioctl<MAGIC, NR, IS_MODERN, D>
{
    /// Tries to interpret a [`RawIoctl`] as this particular ioctl command.
    ///
    /// This method succeeds only if the ioctl command matches. Otherwise, this method returns
    /// `None`.
    pub fn try_from_raw(raw_ioctl: RawIoctl) -> Option<Self> {
        let cmd = IoctlCmd::new(raw_ioctl.cmd);

        if cmd.magic() != MAGIC {
            return None;
        }
        if cmd.nr() != NR {
            return None;
        }

        if IS_MODERN {
            // For modern encoding, the upper 16 bits should contain the size and direction.
            if let Some(size) = D::SIZE
                && cmd.size() != size
            {
                return None;
            }
            if cmd.dir() != D::DIR {
                return None;
            }
        } else {
            // For legacy encoding, the upper 16 bits should be zero.
            if cmd.size() != 0 || cmd.dir() != IoctlDir::None {
                return None;
            }
        }

        Some(Self {
            cmd,
            arg: raw_ioctl.arg,
            _phantom: PhantomData,
        })
    }
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, D: PtrDataSpec>
    Ioctl<MAGIC, NR, IS_MODERN, D>
{
    fn with_data_ptr_unchecked_access<F, R>(&self, f: F) -> R
    where
        F: for<'a> FnOnce(SafePtr<D::Pointee, CurrentUserSpace<'a>>) -> R,
    {
        f(SafePtr::new(current_userspace!(), self.arg))
    }
}

/// No input/output data.
pub struct NoData;

impl DataSpec for NoData {
    const SIZE: Option<u16> = Some(0);
    const DIR: IoctlDir = IoctlDir::None;
}

/// Input-only data.
///
/// `T` describes the data type.
/// `P` describes how the data is passed (by value or by pointer).
pub struct InData<T: ?Sized, P = PassByPtr>(PhantomData<T>, PhantomData<P>);

impl<T, P> DataSpec for InData<T, P> {
    const SIZE: Option<u16> = Some(u16_size_of::<T>());
    const DIR: IoctlDir = IoctlDir::Write;
}

impl<T> PtrDataSpec for InData<T, PassByPtr> {
    type Pointee = T;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, T: Pod>
    Ioctl<MAGIC, NR, IS_MODERN, InData<T, PassByPtr>>
{
    /// Reads the ioctl argument from userspace.
    pub fn read(&self) -> Result<T> {
        Ok(self.with_data_ptr_unchecked_access(|ptr| ptr.read())?)
    }
}

macro_rules! impl_get_by_val_for {
    { $( $ty:ident )* } => {
        $(
            impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool>
                Ioctl<MAGIC, NR, IS_MODERN, InData<$ty, PassByVal>>
            {
                /// Gets the ioctl argument.
                pub fn get(&self) -> $ty {
                    self.arg as $ty
                }
            }
        )*
    };
}

// We can add more types as needed, e.g., `u32`, `i8`.
impl_get_by_val_for! { i32 }

impl DataSpec for InData<[u8]> {
    const SIZE: Option<u16> = None;
    const DIR: IoctlDir = IoctlDir::Write;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool>
    Ioctl<MAGIC, NR, IS_MODERN, InData<[u8]>>
{
    /// Obtains a [`VmReader`] that can read the dynamically-sized ioctl argument from userspace.
    ///
    /// The size of the ioctl argument is specified in [`VmReader::remain`].
    #[expect(dead_code)]
    pub fn with_reader<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(VmReader<'a>) -> Result<R>,
    {
        f(current_userspace!().reader(self.arg, self.cmd.size() as usize)?)
    }
}

/// Output-only data, always passed by pointer.
pub struct OutData<T: ?Sized>(PhantomData<T>);

impl<T> DataSpec for OutData<T> {
    const SIZE: Option<u16> = Some(u16_size_of::<T>());
    const DIR: IoctlDir = IoctlDir::Read;
}

impl<T> PtrDataSpec for OutData<T> {
    type Pointee = T;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, T: Pod>
    Ioctl<MAGIC, NR, IS_MODERN, OutData<T>>
{
    /// Writes the ioctl argument to userspace.
    pub fn write(&self, val: &T) -> Result<()> {
        self.with_data_ptr_unchecked_access(|ptr| ptr.write(val))?;
        Ok(())
    }
}

impl DataSpec for OutData<[u8]> {
    const SIZE: Option<u16> = None;
    const DIR: IoctlDir = IoctlDir::Read;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool>
    Ioctl<MAGIC, NR, IS_MODERN, OutData<[u8]>>
{
    /// Obtains a [`VmWriter`] that can write the dynamically-sized ioctl argument to userspace.
    ///
    /// The size of the ioctl argument is specified in [`VmWriter::avail`].
    #[expect(dead_code)]
    pub fn with_writer<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(VmWriter<'a>) -> Result<R>,
    {
        f(current_userspace!().writer(self.arg, self.cmd.size() as usize)?)
    }
}

/// Input and output data, passed by pointer to a single object.
pub struct InOutData<T>(PhantomData<T>);

impl<T> DataSpec for InOutData<T> {
    const SIZE: Option<u16> = Some(u16_size_of::<T>());
    const DIR: IoctlDir = IoctlDir::ReadWrite;
}

impl<T> PtrDataSpec for InOutData<T> {
    type Pointee = T;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, T: Pod>
    Ioctl<MAGIC, NR, IS_MODERN, InOutData<T>>
{
    /// Reads the ioctl argument from userspace.
    #[expect(dead_code)]
    pub fn read(&self) -> Result<T> {
        self.with_data_ptr(|ptr| Ok(ptr.read()?))
    }

    /// Writes the ioctl argument to userspace.
    pub fn write(&self, val: &T) -> Result<()> {
        self.with_data_ptr(|ptr| Ok(ptr.write(val)?))
    }

    /// Obtains a [`SafePtr`] that can access the ioctl argument in userspace.
    pub fn with_data_ptr<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(SafePtr<T, CurrentUserSpace<'a>>) -> Result<R>,
    {
        self.with_data_ptr_unchecked_access(f)
    }
}

/// A marker that denotes the input is passed by value (i.e., encoded in the ioctl argument).
pub enum PassByVal {}
/// A marker that denotes the input is passed by pointer (i.e., pointed to by the ioctl argument).
pub enum PassByPtr {}

const fn u16_size_of<T>() -> u16 {
    let size = size_of::<T>();
    assert!(
        size <= u16::MAX as usize,
        "the type is too large to fit in an ioctl command"
    );
    size as u16
}

/// Dispatches ioctl commands.
///
/// See [the module-level documentation](self) for how to use this macro and the suggested style to
/// use this macro.
macro_rules! dispatch_ioctl {
    // An empty match.
    (
        match $raw:ident {}
    ) => {
        ()
    };

    // The default branch.
    (
        match $raw:ident {
            _ => $arm:expr $(,)?
        }
    ) => {
        $arm
    };

    // A branch that matches multiple ioctl commands.
    (
        match $raw:ident {
            $ty0:ty $(| $ty1:ty)* => $arm:block $(,)?
            $($rest:tt)*
        }
    ) => {
        if <$ty0>::try_from_raw($raw).is_some()
            $(|| <$ty1>::try_from_raw($raw).is_some())*
        {
            $arm
        } else {
            crate::util::ioctl::dispatch_ioctl!(match $raw { $($rest)* })
        }
    };

    // A branch that matches a single ioctl command.
    (
        match $raw:ident {
            $bind:ident @ $ty:ty => $arm:block $(,)?
            $($rest:tt)*
        }
    ) => {
        if let Some($bind) = <$ty>::try_from_raw($raw) {
            $arm
        } else {
            crate::util::ioctl::dispatch_ioctl!(match $raw { $($rest)* })
        }
    };
}
pub(crate) use dispatch_ioctl;
