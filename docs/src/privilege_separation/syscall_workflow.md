# Case study 1: Common Syscall Workflow

## Problem definition

In a nutshell, the job of an OS is to handle system calls. While system calls may differ greatly in what they do, they share a common syscall handling workflow, which includes at least the following steps.

* User-kernel switching (involving assembly code)
* System call parameter parsing (which has to access CPU registers)
* System call dispatching (needs to _interpret_ integer values to corresponding C types specified by Linux ABI)
* Per-system call handling logic, which often involves accessing user-space memory (pointer dereference)

It seems that each of the steps requires the use of `unsafe` more or less. So the question here is: **is it possible to design a syscall handling framework that has a clear cut between privileged and unprivileged code, allowing the user to handle system calls without `unsafe`?**

The answer is YES. This document describes such a solution.

## To `unsafe`, or not to `unsafe`, that is the question

> The `unsafe` keyword has two uses: to declare the existence of contracts the compiler can't check, and to declare that a programmer has checked that these contracts have been upheld. --- The Rust Unsafe Book

> To isolate unsafe code as much as possible, it’s best to enclose unsafe code within a safe abstraction and provide a safe API. --- The Rust book

Many Rust programmers, sometimes even "professional" ones, do not fully understand when a function should be marked `unsafe` or not. Check out [Kerla OS](https://github.com/nuta/kerla)'s `UserBufWriter` and `UserVAddr` APIs, which is a classic example of _seemingly safe_ APIs that are _unsafe_ in nature.

```rust
impl<'a> SyscallHandler<'a> {
    pub fn sys_clock_gettime(&mut self, clock: c_clockid, buf: UserVAddr) -> Result<isize> {
        let (tv_sec, tv_nsec) = match clock {
            CLOCK_REALTIME => {
                let now = read_wall_clock();
                (now.secs_from_epoch(), now.nanosecs_from_epoch())
            }
            CLOCK_MONOTONIC => {
                let now = read_monotonic_clock();
                (now.secs(), now.nanosecs())
            }
            _ => {
                debug_warn!("clock_gettime: unsupported clock id: {}", clock);
                return Err(Errno::ENOSYS.into());
            }
        };

        let mut writer = UserBufWriter::from_uaddr(buf, size_of::<c_time>() + size_of::<c_long>());
        writer.write::<c_time>(tv_sec.try_into().unwrap())?;
        writer.write::<c_long>(tv_nsec.try_into().unwrap())?;

        Ok(0)
    }
}
```

```rust
/// Represents a user virtual memory address.
///
/// It is guaranteed that `UserVaddr` contains a valid address, in other words,
/// it does not point to a kernel address.
///
/// Futhermore, like `NonNull<T>`, it is always non-null. Use `Option<UserVaddr>`
/// represent a nullable user pointer.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct UserVAddr(usize);

impl UserVAddr {
    pub const fn new(addr: usize) -> Option<UserVAddr> {
        if addr == 0 {
            None
        } else {
            Some(UserVAddr(addr))
        }
    }

    pub fn read<T>(self) -> Result<T, AccessError> {
        let mut buf: MaybeUninit<T> = MaybeUninit::uninit();
        self.read_bytes(unsafe {
            slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, size_of::<T>())
        })?;
        Ok(unsafe { buf.assume_init() })
    }

    pub fn write<T>(self, buf: &T) -> Result<usize, AccessError> {
        let len = size_of::<T>();
        self.write_bytes(unsafe { slice::from_raw_parts(buf as *const T as *const u8, len) })?;
        Ok(len)
    }

    pub fn write_bytes(self, buf: &[u8]) -> Result<usize, AccessError> {
        call_usercopy_hook();
        self.access_ok(buf.len())?;
        unsafe {
            copy_to_user(self.value() as *mut u8, buf.as_ptr(), buf.len());
        }
        Ok(buf.len())
    }
}
```

Interestingly, zCore makes almost exactly the same mistake.

```rust
impl Syscall<'_> {
    /// finds the resolution (precision) of the specified clock clockid, and,
    /// if buffer is non-NULL, stores it in the struct timespec pointed to by buffer
    pub fn sys_clock_gettime(&self, clock: usize, mut buf: UserOutPtr<TimeSpec>) -> SysResult {
        info!("clock_gettime: id={:?} buf={:?}", clock, buf);

        let ts = TimeSpec::now();
        buf.write(ts)?;

        info!("TimeSpec: {:?}", ts);

        Ok(0)
    }
}
```

```rust
pub type UserOutPtr<T> = UserPtr<T, Out>;

/// Raw pointer from user land.
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct UserPtr<T, P: Policy>(*mut T, PhantomData<P>);

impl<T, P: Policy> From<usize> for UserPtr<T, P> {
    fn from(ptr: usize) -> Self {
        UserPtr(ptr as _, PhantomData)
    }
}

impl<T, P: Write> UserPtr<T, P> {
    /// Overwrites a memory location with the given `value`
    /// **without** reading or dropping the old value.
    pub fn write(&mut self, value: T) -> Result<()> {
        self.check()?; // check non-nullness and alignment
        unsafe { self.0.write(value) };
        Ok(())
    }
}
```

The examples reveal two important considerations in designing Asterinas:
1. Exposing _truly_ safe APIs. The privileged OS core must expose _truly safe_ APIs: however buggy or silly the unprivileged OS components may be written, they must _not_ cause undefined behaviors.
2. Handling _arbitrary_ pointers safely. The safe API of the OS core must provide a safe way to deal with arbitrary pointers.

With the two points in mind, let's get back to our main goal of privilege separation.

## Code organization with privilege separation

Our first step is to separate privileged and unprivileged code in the codebase of Asterinas. For our purpose of demonstrating a syscall handling framework, a minimal codebase may look like the following.

```text
.
├── asterinas
│   ├── src
│   │   └── main.rs
│   └── Cargo.toml
├── aster-core
│   ├── src
│   │   ├── lib.rs
│   │   ├── syscall_handler.rs
│   │   └── vm
│   │       ├── vmo.rs
│   │       └── vmar.rs
│   └── Cargo.toml
├── aster-core-libs
│   ├── linux-abi-types
│   │   ├── src
│   │   │   └── lib.rs
│   │   └── Cargo.toml
│   └── pod
│       ├── src
│       │   └── lib.rs
│       └── Cargo.toml
├── aster-comps
│   └── linux-syscall
│       ├── src
│       │   └── lib.rs
│       └── Cargo.toml
└── aster-comp-libs
    └── linux-abi
        ├── src
        │   └── lib.rs
        └── Cargo.toml 
```

The ultimate build target of the codebase is the `asterinas` crate, which is an OS kernel that consists of a privileged OS core (crate `aster-core`) and multiple OS components (the crates under `aster-comps/`).

For the sake of privilege separation, only crate `asterinas` and `aster-core` along with the crates under `aster-core-libs` are allowed to use the `unsafe` keyword. To the contrary, the crates under `aster-comps/` along with their dependent crates under `aster-comp-libs/` are not allowed to use `unsafe` directly; they may only borrow the superpower of `unsafe` by using the safe API exposed by `aster-core` or the crates under `aster-core-libs`. To summarize, the memory safety of the OS only relies on a small and well-defined TCB that constitutes the `asterinas` and `aster-core` crate plus the crates under `aster-core-libs/`.

Under this setting, all implementation of system calls goes to the `linux-syscall` crate. We are about to show that the _safe_ API provided by `aster-core` is powerful enough to enable the _safe_ implementation of `linux-syscall`.

## Crate `aster-core`

For our purposes here, the two most relevant APIs provided by `aster-core` is the abstraction for syscall handlers and virtual memory (VM).

### Syscall handlers

The `SyscallHandler` abstraction enables the OS core to hide the low-level, architectural-dependent aspects of syscall handling workflow (e.g., user-kernel switching and CPU register manipulation) and allow the unprivileged OS components to implement system calls.

```rust
// file: aster-core/src/syscall_handler.rs

pub trait SyscallHandler {
    fn handle_syscall(&self, ctx: &mut SyscallContext);
}

pub struct SyscallContext { /* cpu states */ }

pub fn set_syscall_handler(handler: &'static dyn SyscallHandler) {
    todo!("set HANDLER")
}

pub(crate) fn syscall_handler() -> &'static dyn SyscallHandler {
    HANDLER
}

static mut HANDLER: &'static dyn SyscallHandler = &DummyHandler;

struct DummyHandler;

impl SyscallHandler for DummyHandler {
    fn handle_syscall(&self, ctx: &mut UserContext) {
        ctx.set_retval(-Errno::ENOSYS);
    }
}
```

### VM capabilities

The OS core provides two abstractions related to virtual memory management.
* _Virtual Memory Address Region (VMAR)_. A VMAR represents a range of virtual address space. In essense, VMARs abstract away the architectural details regarding page tables.
* _Virtual Memory Pager (VMP)_. A VMP represents a range of memory pages (yes, the memory itself, not the address space). VMPs encapsulates the management of physical memory pages and enable on-demand paging.

Both VMARs and VMPs are _privileged_ as they need to have direct access to page tables and physical memory, which demands the use of `unsafe`.

These two abstractions are adopted from similar concepts in zircon ([Virtual Memory Address Regions (VMARs)](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/vm_address_region) and [Virtual Memory Object (VMO)](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/vm_object)), also implemented by zCore.

Interestingly, both VMARs and VMPs are [capabilities](../capabilities/README.md),
an important concept that we will elaborate on later. Basically, they are capabilities as they satisfy the following two properties of *non-forgeability* and *monotonicity*. This is because 1) a root VMAR or VMP can only be created via a few well-defined APIs exposed by the OS core, and 2) a child VMAR o VMP can only be derived from an existing VMAR or VMP with more limited access to resources (e.g., a subset of the parent's address space or memory pages or access permissions).

##  Crate `linux-syscall`

Here we demonstrate how to leverage the APIs of `ksos-core` to implement system calls with safe Rust code in crate `linux-syscall`.

```rust
// file: aster-comps/linux-syscall/src/lib.rs
use aster_core::{SyscallContext, SyscallHandler, Vmar};
use linux_abi::{SyscallNum::*, UserPtr, RawFd, RawTimeVal, RawTimeZone};

pub struct SampleHandler;

impl SyscallHandler for SampleHandler {
    fn handle_syscall(&self, ctx: &mut SyscallContext) {
        let syscall_num = ctx.num();
        let (a0, a1, a2, a3, a4, a5) = ctx.args();
        match syscall_num {
            SYS_GETTIMEOFDAY => {
                let tv_ptr = UserPtr::new(a0 as usize);
                let tz_ptr = UserPtr::new(a1 as usize);
                let res = self.sys_gettimeofday(tv_ptr, );
                todo!("set retval according to res");
            }
            SYS_SETRLIMIT => {
                let resource = a0 as u32;
                let rlimit_ptr = UserPtr::new(a1 as usize);
                let res = self.sys_setrlimit(resource, rlimit_ptr);
                todo!("set retval according to res");
            }
            _ => {
                ctx.set_retval(-Errno::ENOSYS)
            }
        };
    }
}

impl SampleHandler {
    fn sys_gettimeofday(&self, tv_ptr: UserPtr<RawTimeVal>, _tz_ptr: UserPtr<RawTimeZone>) -> Result<()> {
        if tv_ptr.is_null() {
            return Err(Errno::EINVAL);
        }
        
        // Get the VMAR of this process
        let vmar = self.thread().process().vmar();
        let tv_val: RawTimeVal = todo!("get current time");
        // Write a value according to the arbitrary pointer
        // is safe because
        // 1) the vmar refers to the memory in the user space;
        // 2) the read_slice method checks memory validity (no page faults);
        //
        // Note that the vmar of the OS kernel cannot be 
        // manipulated directly by any OS components outside
        // the OS core.
        vmar.write_val(tv_ptr, tv_val)?;
        Ok(())
    }

    fn sys_setrlimit(&self, resource: u32, rlimit_ptr: UserPtr<RawRlimit>) -> Result<u32> {
        if rlimit_ptr.is_null() {
            return Err(Errno::EINVAL);
        }

        let vmar = self.thread().process().vmar();
        // Read a value according to the arbitrary pointer is safe
        // due to reasons similar to the above code, but with one
        // addition reason: the value is of a type `T: Pod`, i.e.,
        // Plain Old Data (POD).
        let new_rlimit = vmar.read_val::<RawRlimit>(rlimit_ptr)?;
        todo!("use the new rlimit value")
    }
}
```

## Crate `pod`

This crate defines a marker trait `Pod`, which represents plain-old data.

```rust
/// file: aster-core-libs/pod/src/lib.rs

/// A marker trait for plain old data (POD).
///
/// A POD type `T:Pod` supports converting to and from arbitrary
/// `mem::size_of::<T>()` bytes _safely_.
/// For example, simple primitive types like `u8` and `i16` 
/// are POD types. But perhaps surprisingly, `bool` is not POD
/// because Rust compiler makes implicit assumption that 
/// a byte of `bool` has a value of either `0` or `1`.
/// Interpreting a byte of value `3` has a `bool` value has
/// undefined behavior.
///
/// # Safety
///
/// Marking a non-POD type as POD may cause undefined behaviors.
pub unsafe trait Pod: Copy + Sized {
    fn new_from_bytes(bytes: &[u8]) -> Self {
        *Self::from_bytes(bytes)
    }
    
    fn from_bytes(bytes: &[u8]) -> &Self {
        // Ensure the size and alignment are ok
        assert!(bytes.len() == core::mem::size_of::<Self>());
        assert!((bytes as *const u8 as usize) % core::mem::align_of::<Self>() == 0);
        
        unsafe {
            core::mem::transmute(bytes)
        }
    }

    fn from_bytes_mut(bytes: &[u8]) -> &mut Self {
        // Ensure the size and alignment are ok
        assert!(bytes.len() == core::mem::size_of::<Self>());
        assert!((bytes as *const u8 as usize) % core::mem::align_of::<Self>() == 0);
        
        unsafe {
            core::mem::transmute(bytes)
        }
    }

    fn as_bytes(&self) -> &[u8] {
        let ptr = self as *const u8;
        let len = core::mem::size_of::<Self>();
        unsafe {
            core::slice::from_raw_parts(ptr, len)
        }
    }

    fn as_bytes_mut(&mut self) -> &mut [u8] {
        let ptr = self as *mut u8;
        let len = core::mem::size_of::<Self>();
        unsafe {
            core::slice::from_raw_parts_mut(ptr, len)
        }
    }
}

macro_rule! impl_pod_for {
    (/* define the input */) => { /* define the expansion */ }
}

impl_pod_for!(
    u8, u16, u32, u64,
    i8, i16, i32, i64,
);

unsafe impl<T: Pod, const N> [T; N] for Pod {}
```

## Crate `linux-abi-type`

```rust
// file: aster-core-libs/linux-abi-types
use pod::Pod;

pub type RawFd = i32;

pub struct RawTimeVal {
    sec: u64,
    usec: i64,
}

unsafe impl Pod for RawTimeVal {}
```

## Crate `linux-abi`

```rust
// file: aster-comp-libs/linux-abi
pub use linux_abi_types::*;

pub enum SyscallNum {
    Read = 0,
    Write = 1,
    /* ... */
}
```

## Wrap up

I hope that this document has convinced you that with the right abstractions (e.g., `SyscallHandler`, `Vmar`, `Vmp`, and `Pod`), it is possible to write system calls---at least, the main system call workflow---without _unsafe_ Rust.