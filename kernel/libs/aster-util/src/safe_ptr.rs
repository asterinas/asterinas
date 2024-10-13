// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Debug, marker::PhantomData};

use aster_rights::{Dup, Exec, Full, Read, Signal, TRightSet, TRights, Write};
use aster_rights_proc::require;
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{Daddr, DmaStream, HasDaddr, HasPaddr, Paddr, PodOnce, VmIo, VmIoOnce},
    Pod, Result,
};

/// Safe pointers.
///
/// # Overview
///
/// Safe pointers allows using pointers to access memory without
/// unsafe code, which is a key enabler for writing device drivers in safe Rust.
///
/// To ensure its soundness, safe pointers (`SafePtr<T, M, _>`) have to be
/// more restricted than raw pointers (`*const T` or `*mut T`).
/// More specifically, there are three major restrictions.
///
/// 1. A safe pointer can only refer to a value of a POD type `T: Pod`,
///    while raw pointers can do to a value of any type `T`.
/// 2. A safe pointer can only refer to an address within a virtual memory object
///    of type `M: VmIo` (e.g., VMAR and VMO), while raw pointers can do to
///    an address within any virtual memory space.
/// 3. A safe pointer only allows one to copy values to/from the target address,
///    while a raw pointer allows one to borrow an immutable or mutable reference
///    to the target address.
///
/// The expressiveness of safe pointers, although being less than that of
/// raw pointers, is sufficient for our purpose of writing an OS kernel in safe
/// Rust.
///
/// In addition, safe pointers `SafePtr<T, M, R>` are associated with access
/// rights, which are encoded statically with type `R: TRights`.
///
/// # Examples
///
/// ## Constructing a safe pointer
///
/// An instance of `SafePtr` can be created with a VM object and an address
/// within the VM object.
///
/// ```
/// let u32_ptr: SafePtr<u32, Vec<u8>, _> = {
///     let vm_obj = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
///     let addr = 16;
///     SafePtr::new(vm_obj, addr)
/// };
/// ```
///
/// The generic parameter `M` of `SafePtr<_, M, _>` must implement the `VmIo`
/// trait. The most important `VmIo` types are `Vmar`, `Vmo`, `IoMem`, and
/// `Frame`. The blanket implementations of `VmIo` also include pointer-like
/// types that refer to a `VmIo` type. Some examples are `&Vmo`, `Box<Vmar>`,
/// and `Arc<IoMem>`.
///
/// The safe pointer itself does not and cannot guarantee that its address is valid.
/// This is because different VM objects may interpret addresses differently
/// and each VM object can have different restrictions for valid addresses.
/// So the detection of invalid addresses is delayed to the time when the
/// pointers are actually read from or written to.
///
/// Initially, a newly-created safe pointer has all access rights.
///
/// ## Reading and writing a safe pointer
///
/// The value pointed to by a safe pointer can be read or written with the
/// `read` or `write` method. Both methods may return errors. The possible reasons
/// of error are determined by the underlying VM objects.
///
/// ```
/// u32_ptr.write(1234).unwrap();
/// assert!(u32_ptr.read().unwrap() == 1234);
/// ```
///
/// ## Manipulating a safe pointer
///
/// The address of a safe pointer can be obtained by the `addr` method.
/// The address can be updated by assigning a new value with the `set_addr` method
/// or updated incrementally through methods like `add`, `offset`, `byte_addr`,
/// `byte_offset`.
///  
/// The VM object of a safe pointer can also be obtained or updated through the
/// `vm` and `set_vm` methods. A new safe pointer that is backed by the same
/// VM object of an existing safe pointer can be obtained through the `borrow_vm`
/// method.
///
/// As an example, the code below shows how the `add` and `borrow_vm` methods
/// can be used together to to iterate all values pointed to by an array pointer.
///
/// ```
/// fn collect_values<T>(array_ptr: &SafePtr<T, M, _>, array_len: usize) -> Vec<T> {
///     let mut curr_ptr: SafePtr<T, &M, _> = array_ptr.borrow_vm();
///     (0..array_len)
///         .iter()
///         .map(|_| {
///             let val = curr_ptr.read().unwrap();
///             curr_ptr.add(1);
///             val
///         })
///         .collect()
/// }
/// ```
///
/// The data type of a safe pointer can be converted with the `cast` method.
///
/// ```rust
/// let u8_ptr: SafePtr<u8, _, _> = u32_ptr.cast();
/// ```
///
/// ## Reading and writing the fields of a struct
///
/// Given a safe pointer that points to a struct (say, `Foo`), one can read
/// the value of its field as follows.
///
/// ```
/// pub struct Foo {
///     first: u64,
///     second: u32,
/// }
///
/// fn read_second_field<M: VmIo>(ptr: &SafePtr<Foo, M, _>) -> u32 {
///     let field_ptr = ptr
///         .borrow_vm()
///         .byte_add(offset_of!(Foo, second) as usize)
///         .cast::<u32>();
///     field_ptr.read().unwrap()
/// }
/// ```
///
/// But this coding pattern is too tedius for such a common task.
/// To make the life of users easier, we provide a convenient macro named
/// `field_ptr`, which can be used to obtain the safe pointer of a field from
/// that of its containing struct.
///
/// ```
/// fn read_second_field<M: VmIo>(ptr: &SafePtr<Foo, M, _>) -> u32 {
///     let field_ptr = field_ptr!(ptr, Foo, second);
///     field_ptr.read().unwrap()
/// }
/// ```
///
/// # Access rights
///
/// A safe pointer may have a combination of three access rights:
/// Read, Write, and Dup.
pub struct SafePtr<T, M, R = Full> {
    offset: usize,
    vm_obj: M,
    rights: R,
    phantom: PhantomData<T>,
}

impl<T, M> SafePtr<T, M> {
    /// Create a new instance.
    ///
    /// # Access rights
    ///
    /// The default access rights of a new instance are `Read`, `Write`, and
    /// `Dup`.
    pub fn new(vm_obj: M, offset: usize) -> Self {
        Self {
            vm_obj,
            offset,
            rights: TRightSet(<TRights![Dup, Read, Write, Exec, Signal]>::new()),
            phantom: PhantomData,
        }
    }
}

impl<T, M: HasPaddr, R> SafePtr<T, M, R> {
    pub fn paddr(&self) -> Paddr {
        self.vm_obj.paddr() + self.offset
    }
}

// =============== Read and write methods ==============
impl<T: Pod, M: VmIo, R: TRights> SafePtr<T, M, TRightSet<R>> {
    /// Read the value from the pointer.
    ///
    /// # Access rights
    ///
    /// This method requires the Read right.
    #[require(R > Read)]
    pub fn read(&self) -> Result<T> {
        self.vm_obj.read_val(self.offset)
    }

    /// Read a slice of values from the pointer.
    ///
    /// # Access rights
    ///
    /// This method requires the Read right.
    #[require(R > Read)]
    pub fn read_slice(&self, slice: &mut [T]) -> Result<()> {
        self.vm_obj.read_slice(self.offset, slice)
    }

    /// Overwrite the value at the pointer.
    ///
    /// # Access rights
    ///
    /// This method requires the Write right.
    #[require(R > Write)]
    pub fn write(&self, val: &T) -> Result<()> {
        self.vm_obj.write_val(self.offset, val)
    }

    /// Overwrite a slice of values at the pointer.
    ///
    /// # Access rights
    ///
    /// This method requires the Write right.
    #[require(R > Write)]
    pub fn write_slice(&self, slice: &[T]) -> Result<()> {
        self.vm_obj.write_slice(self.offset, slice)
    }
}

// =============== Read and write methods ==============
impl<T: PodOnce, M: VmIoOnce, R: TRights> SafePtr<T, M, TRightSet<R>> {
    /// Reads the value from the pointer using one non-tearing instruction.
    ///
    /// # Access rights
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn read_once(&self) -> Result<T> {
        self.vm_obj.read_once(self.offset)
    }

    /// Overwrites the value at the pointer using one non-tearing instruction.
    ///
    /// # Access rights
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn write_once(&self, val: &T) -> Result<()> {
        self.vm_obj.write_once(self.offset, val)
    }
}

// =============== Address-related methods ==============
impl<T, M, R> SafePtr<T, M, R> {
    pub const fn is_aligned(&self) -> bool {
        self.offset % core::mem::align_of::<T>() == 0
    }

    /// Increase the address in units of bytes occupied by the generic T.
    pub fn add(&mut self, count: usize) {
        let offset = count * core::mem::size_of::<T>();
        self.offset += offset;
    }

    /// Increase or decrease the address in units of bytes occupied by the generic T.
    pub fn offset(&mut self, count: isize) {
        let offset = count * core::mem::size_of::<T>() as isize;
        if count >= 0 {
            self.offset += offset as usize;
        } else {
            self.offset -= offset as usize;
        }
    }

    /// Increase the address in units of bytes.
    pub fn byte_add(&mut self, bytes: usize) {
        self.offset += bytes;
    }

    /// Increase or decrease the address in units of bytes.
    pub fn byte_offset(&mut self, bytes: isize) {
        if bytes >= 0 {
            self.offset += bytes as usize;
        } else {
            self.offset -= (-bytes) as usize;
        }
    }
}

// =============== VM object-related methods ==============
impl<T, M, R> SafePtr<T, M, R> {
    pub const fn vm(&self) -> &M {
        &self.vm_obj
    }

    pub fn set_vm(&mut self, vm_obj: M) {
        self.vm_obj = vm_obj;
    }
}

// =============== VM object-related methods ==============
impl<T, M, R: Clone> SafePtr<T, M, R> {
    /// Construct a new SafePtr which will point to the same address
    pub fn borrow_vm(&self) -> SafePtr<T, &M, R> {
        let SafePtr {
            offset: addr,
            vm_obj,
            rights,
            ..
        } = self;
        SafePtr {
            offset: *addr,
            vm_obj,
            rights: rights.clone(),
            phantom: PhantomData,
        }
    }
}

// =============== Type conversion methods ==============
impl<T, M, R> SafePtr<T, M, R> {
    /// Cast the accessed structure into a new one, which is usually used when accessing a field in a structure.
    pub fn cast<U>(self) -> SafePtr<U, M, R> {
        let SafePtr {
            offset: addr,
            vm_obj,
            rights,
            ..
        } = self;
        SafePtr {
            offset: addr,
            vm_obj,
            rights,
            phantom: PhantomData,
        }
    }
}

// =============== Type conversion methods ==============
impl<T, M, R: TRights> SafePtr<T, M, TRightSet<R>> {
    /// Construct a new SafePtr and restrict the rights of it.
    ///
    /// # Access rights
    ///
    /// This method requires the target rights to be a subset of the current rights.
    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> SafePtr<T, M, TRightSet<R1>> {
        let SafePtr {
            offset: addr,
            vm_obj,
            ..
        } = self;
        SafePtr {
            offset: addr,
            vm_obj,
            rights: TRightSet(R1::new()),
            phantom: PhantomData,
        }
    }
}

impl<T, M: HasDaddr, R> HasDaddr for SafePtr<T, M, R> {
    fn daddr(&self) -> Daddr {
        self.offset + self.vm_obj.daddr()
    }
}

impl<T, R> SafePtr<T, DmaStream, R> {
    /// Synchronize the object in the streaming DMA mapping
    pub fn sync(&self) -> Result<()> {
        self.vm_obj
            .sync(self.offset..self.offset + core::mem::size_of::<T>())
    }
}

#[inherit_methods(from = "(*self)")]
impl<T, R> SafePtr<T, &DmaStream, R> {
    pub fn sync(&self) -> Result<()>;
}

#[require(R > Dup)]
impl<T, M: Clone, R: TRights> Clone for SafePtr<T, M, TRightSet<R>> {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            vm_obj: self.vm_obj.clone(),
            rights: self.rights,
            phantom: PhantomData,
        }
    }
}

#[require(R > Dup)]
impl<T, M: crate::dup::Dup, R: TRights> crate::dup::Dup for SafePtr<T, M, TRightSet<R>> {
    fn dup(&self) -> Result<Self> {
        let duplicated = Self {
            offset: self.offset,
            vm_obj: self.vm_obj.dup()?,
            rights: self.rights,
            phantom: PhantomData,
        };
        Ok(duplicated)
    }
}

impl<T, M: Debug, R> Debug for SafePtr<T, M, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SafePtr")
            .field("offset", &self.offset)
            .field("vm_obj", &self.vm_obj)
            .finish()
    }
}

/// Create a safe pointer for the field of a struct.
#[macro_export]
macro_rules! field_ptr {
    ($ptr:expr, $type:ty, $($field:tt)+) => {{
        use ostd::offset_of;
        use aster_util::safe_ptr::SafePtr;

        #[inline]
        fn new_field_ptr<T, M, R: Clone, U>(
            container_ptr: &SafePtr<T, M, R>,
            field_offset: *const U
        ) -> SafePtr<U, &M, R>
        {
            let mut ptr = container_ptr.borrow_vm();
            ptr.byte_add(field_offset as usize);
            ptr.cast()
        }

        let field_offset = offset_of!($type, $($field)*);
        new_field_ptr($ptr, field_offset)
    }}
}
