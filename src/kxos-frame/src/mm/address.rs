use core::ops::{Add, AddAssign, Sub, SubAssign};

use alloc::fmt;

use crate::config::{PAGE_SIZE, PHYS_OFFSET};

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
#[repr(transparent)]
pub struct PhysAddr(pub usize);

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
#[repr(transparent)]
pub struct VirtAddr(pub usize);

pub const fn phys_to_virt(pa: usize) -> usize {
    pa + PHYS_OFFSET
}

pub const fn virt_to_phys(va: usize) -> usize {
    va - PHYS_OFFSET
}

impl PhysAddr {
    pub const fn kvaddr(self) -> VirtAddr {
        VirtAddr(phys_to_virt(self.0))
    }
    pub const fn align_down(self) -> Self {
        Self(align_down(self.0))
    }
    pub const fn align_up(self) -> Self {
        Self(align_up(self.0))
    }
    pub const fn page_offset(self) -> usize {
        page_offset(self.0)
    }
    pub const fn is_aligned(self) -> bool {
        is_aligned(self.0)
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("PhysAddr")
            .field(&format_args!("{:#x}", self.0))
            .finish()
    }
}

impl fmt::Binary for PhysAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Binary::fmt(&self.0, f)
    }
}

impl fmt::LowerHex for PhysAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

impl fmt::Octal for PhysAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Octal::fmt(&self.0, f)
    }
}

impl fmt::UpperHex for PhysAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, f)
    }
}

impl fmt::Pointer for PhysAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Pointer::fmt(&(self.0 as *const ()), f)
    }
}

impl Add<usize> for PhysAddr {
    type Output = Self;
    #[inline]
    fn add(self, rhs: usize) -> Self::Output {
        PhysAddr(self.0 + rhs)
    }
}

impl AddAssign<usize> for PhysAddr {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        *self = *self + rhs;
    }
}

impl Sub<usize> for PhysAddr {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: usize) -> Self::Output {
        PhysAddr(self.0 - rhs)
    }
}

impl SubAssign<usize> for PhysAddr {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        *self = *self - rhs;
    }
}

impl Sub<PhysAddr> for PhysAddr {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: PhysAddr) -> Self::Output {
        self.0.checked_sub(rhs.0).unwrap().try_into().unwrap()
    }
}

impl VirtAddr {
    pub const fn as_ptr(self) -> *mut u8 {
        self.0 as _
    }
    pub const fn align_down(self) -> Self {
        Self(align_down(self.0))
    }
    pub const fn align_up(self) -> Self {
        Self(align_up(self.0))
    }
    pub const fn page_offset(self) -> usize {
        page_offset(self.0)
    }
    pub const fn is_aligned(self) -> bool {
        is_aligned(self.0)
    }
}

impl VirtAddr {
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.0 as *mut u8, 4096) }
    }
    pub fn get_ref<T>(&self) -> &'static T {
        unsafe { (self.0 as *const T).as_ref().unwrap() }
    }
    pub fn get_mut<T>(&self) -> &'static mut T {
        unsafe { (self.0 as *mut T).as_mut().unwrap() }
    }
}

impl fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("VirtAddr")
            .field(&format_args!("{:#x}", self.0))
            .finish()
    }
}

impl fmt::Binary for VirtAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Binary::fmt(&self.0, f)
    }
}

impl fmt::LowerHex for VirtAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

impl fmt::Octal for VirtAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Octal::fmt(&self.0, f)
    }
}

impl fmt::UpperHex for VirtAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, f)
    }
}

impl fmt::Pointer for VirtAddr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Pointer::fmt(&(self.0 as *const ()), f)
    }
}

impl Add<usize> for VirtAddr {
    type Output = Self;
    #[inline]
    fn add(self, rhs: usize) -> Self::Output {
        VirtAddr(self.0 + rhs)
    }
}

impl AddAssign<usize> for VirtAddr {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        *self = *self + rhs;
    }
}

impl Sub<usize> for VirtAddr {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: usize) -> Self::Output {
        VirtAddr(self.0 - rhs)
    }
}

impl SubAssign<usize> for VirtAddr {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        *self = *self - rhs;
    }
}

impl Sub<VirtAddr> for VirtAddr {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: VirtAddr) -> Self::Output {
        self.0.checked_sub(rhs.0).unwrap().try_into().unwrap()
    }
}

pub const fn align_down(p: usize) -> usize {
    p & !(PAGE_SIZE - 1)
}

pub const fn align_up(p: usize) -> usize {
    (p + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

pub const fn page_offset(p: usize) -> usize {
    p & (PAGE_SIZE - 1)
}

pub const fn is_aligned(p: usize) -> bool {
    page_offset(p) == 0
}
