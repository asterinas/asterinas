#[cfg(not(test))]
pub(crate) use log::{debug, info, warn};
#[cfg(test)]
pub(crate) use std::{println as debug, println as info, println as warn};

// Block IO Layer
pub(crate) use block_io::{
    bid::{BlockId, BLOCK_SIZE},
    bio::{Bio, BioBuf},
    block_device::{BlockDevice, BlockDeviceExt},
};

// Common components
pub(crate) use crate::error::{Error, Result};
pub(crate) use crate::traits::PageCache;
pub(crate) use crate::utils::{align_up, Dirty, IsPowerOf};
pub(crate) use alloc::boxed::Box;
pub(crate) use alloc::collections::BTreeMap;
pub(crate) use alloc::string::String;
pub(crate) use alloc::sync::{Arc, Weak};
pub(crate) use alloc::vec;
pub(crate) use alloc::vec::Vec;
pub(crate) use bitflags::bitflags;
pub(crate) use core::fmt::Debug;
pub(crate) use core::iter::Iterator;
pub(crate) use core::ops::Range;
pub(crate) use core::ops::{Deref, DerefMut};
pub(crate) use int_to_c_enum::TryFromInt;
pub(crate) use mem_storage::{GenericIo, MemStorage};
pub(crate) use pod::Pod;

#[cfg(feature = "jinux")]
pub(crate) use jinux_frame::sync::RwLock;
#[cfg(not(feature = "jinux"))]
pub(crate) use spin::RwLock;
