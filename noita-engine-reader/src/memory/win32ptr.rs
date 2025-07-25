use core::fmt;
use std::{fmt::Debug, io, marker::PhantomData, mem::size_of, panic::Location};

use serde::ser::{Serialize, Serializer};
use zerocopy::{FromBytes, IntoBytes};

use crate::memory::debug_type;

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PtrReadable)]
#[repr(transparent)]
pub struct RawPtr(u32);

impl RawPtr {
    pub const fn of(addr: u32) -> Self {
        Self(addr)
    }

    pub const fn cast<T>(self) -> Ptr<T> {
        Ptr::of(self.0)
    }

    pub const fn addr(self) -> u32 {
        self.0
    }

    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    pub const fn offset(self, offset: i32) -> Self {
        Self::of((self.0 as i32 + offset) as u32)
    }

    pub fn read_multiple<T: Pod>(self, proc: &ProcessRef, len: u32) -> io::Result<Vec<T>> {
        proc.read_multiple(self.0, len)
    }

    pub fn read<T: Pod>(self, proc: &ProcessRef) -> io::Result<T> {
        proc.read(self.0)
    }
}

impl Debug for RawPtr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            write!(f, "NULL")
        } else {
            write!(f, "0x{:08x}", self.0)
        }
    }
}

impl From<u32> for RawPtr {
    fn from(addr: u32) -> Self {
        Self::of(addr)
    }
}

#[derive(FromBytes, IntoBytes)]
#[repr(transparent)]
pub struct Ptr<T> {
    raw: RawPtr,
    _phantom: PhantomData<T>,
}

impl<T> Ptr<T> {
    pub const fn of(addr: u32) -> Self {
        Self {
            raw: RawPtr::of(addr),
            _phantom: PhantomData,
        }
    }

    pub const fn offset(self, offset: i32) -> Self {
        Self::of((self.raw.addr() as i32 + offset * size_of::<T>() as i32) as u32)
    }
}

impl<T> Ptr<T> {
    pub const fn addr(self) -> u32 {
        self.raw.addr()
    }

    pub const fn is_null(&self) -> bool {
        self.addr() == 0
    }

    pub const fn raw(self) -> RawPtr {
        RawPtr::of(self.addr())
    }
}

impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Ptr<T> {}

impl<T> PartialEq for Ptr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for Ptr<T> {}

impl<T> Debug for Ptr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.raw.is_null() {
            write!(f, "NULL")
        } else {
            write!(f, "{:?} as {}", self.raw, debug_type::<T>())
        }
    }
}

impl<T> Serialize for Ptr<T>
where
    T: PtrReadable + Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Only try to dereference if we have a process context and pointer is not null
        crate::memory::DEBUG_PROCESS.with_borrow(|proc| {
            if let Some(proc) = proc.as_ref() {
                if self.raw.is_null() {
                    return serializer.serialize_none();
                }
                match self.read(proc) {
                    Ok(val) => val.serialize(serializer),
                    Err(_) => serializer.serialize_none(),
                }
            } else {
                serializer.serialize_u32(self.raw.addr())
            }
        })
    }
}

impl<T> From<u32> for Ptr<T> {
    fn from(addr: u32) -> Self {
        Self::of(addr)
    }
}

// pointers themselves are readable through pointers
impl<T: 'static> PtrReadable for Ptr<T> {}

impl<T: PtrReadable> MemoryStorage for Ptr<T> {
    type Value = T;

    #[track_caller]
    fn read(&self, proc: &ProcessRef) -> io::Result<Self::Value> {
        if self.raw.is_null() {
            let loc = Location::caller();
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Reading a NULL pointer at {loc}"),
            ))
        } else {
            self.raw.read(proc)
        }
    }
}

// Sadly, this is a specialization, for it to work we need a blanket noop impl
// for MemoryStorage, which would conflict with this
//
// impl<T: MemoryStorage> MemoryStorage for Ptr<T> {
//     type Value = T::Value;

//     fn read(&self, proc: &ProcessRef) -> io::Result<Self::Value> {
//         self.raw.read_at::<T>(BASE, proc)?.read(proc)
//     }
// }

#[derive(FromBytes, IntoBytes, Clone, Copy)]
#[repr(transparent)]
pub struct Vftable {
    pub ptr: RawPtr,
}

impl Vftable {
    pub fn get_rtti_name(&self, proc: &ProcessRef) -> std::io::Result<String> {
        let name = self
            .ptr
            .offset(-4) // meta pointer is behind the vftable
            .read::<RawPtr>(proc)?
            .offset(12) // skip signature, offset and cdOffset
            .read::<RawPtr>(proc)?
            .offset(8); // skip type_info::vftable and spare

        CString::from(name).read(proc)
    }
}

impl Debug for Vftable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(s) =
            DEBUG_PROCESS.with_borrow(|proc| proc.as_ref().and_then(|h| self.get_rtti_name(h).ok()))
        {
            return f
                .debug_struct("Vftable")
                .field("rtti_name", &format_args!("{s:?}"))
                .field("ptr", &format_args!("{:?}", self.ptr))
                .finish();
        }

        f.debug_tuple("Vftable")
            .field(&format_args!("{:?}", self.ptr))
            .finish()
    }
}
