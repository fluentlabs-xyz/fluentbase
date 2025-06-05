use solana_account_info::AccountInfo;
use solana_rbpf::{
    error::ProgramResult,
    memory_region::{AccessType, MemoryMapping},
};
use std::any::type_name;

pub const FIXED_MACHINE_WORD_BYTE_SIZE: usize = 8;
pub const FIXED_PTR_BYTE_SIZE: usize = FIXED_MACHINE_WORD_BYTE_SIZE;
pub const FAT_PTR64_ELEM_BYTE_SIZE: usize = FIXED_MACHINE_WORD_BYTE_SIZE;
pub const SLICE_FAT_PTR64_SIZE_BYTES: usize = FAT_PTR64_ELEM_BYTE_SIZE * 2;
pub const STABLE_VEC_FAT_PTR64_BYTE_SIZE: usize = FAT_PTR64_ELEM_BYTE_SIZE * 3;

#[macro_export]
macro_rules! typ_align {
    ($typ:ty) => {
        core::mem::align_of::<$typ>()
    };
}

#[macro_export]
macro_rules! typ_name {
    ($typ:ty) => {
        core::any::type_name::<$typ>()
    };
}

#[macro_export]
macro_rules! typ_size {
    ($typ:ty) => {
        core::mem::size_of::<$typ>()
    };
}

#[macro_export]
macro_rules! println_typ_size {
    ($typ:ty) => {
        println!(
            "size_of::<{}>() = {}",
            $crate::typ_name!($typ),
            $crate::typ_size!($typ)
        )
    };
}

#[inline(always)]
fn validate_typecast<T: Clone>(data: &[u8]) {
    let data = data.as_ref();
    let type_name = crate::typ_name!(T);
    if data.len() < crate::typ_size!(T) {
        panic!("failed to typecase to {}: invalid size", type_name);
    }

    let ptr = data.as_ptr() as *const T;

    // Check alignment
    if (ptr as usize) % crate::typ_align!(T) != 0 {
        panic!("failed to typecase to {}: misaligned", type_name);
    }
}

#[inline(always)]
pub fn typecast_bytes<T: Clone>(data: &[u8]) -> &T {
    validate_typecast::<T>(data);

    unsafe { &*(data.as_ptr() as *const T) }
}

#[inline(always)]
pub fn typecast_bytes_mut<T: Clone>(data: &[u8]) -> &mut T {
    validate_typecast::<T>(data);

    unsafe { &mut *(data.as_ptr() as *mut T) }
}

#[derive(Clone)]
pub struct MemoryMappingHelper<'a> {
    memory_mapping: Option<&'a MemoryMapping<'a>>,
    access_type: Option<AccessType>,
}

impl Default for MemoryMappingHelper<'_> {
    fn default() -> Self {
        MemoryMappingHelper::new(None, None)
    }
}

impl<'a> MemoryMappingHelper<'a> {
    pub fn new(
        memory_mapping: Option<&'a MemoryMapping<'a>>,
        access_type: Option<AccessType>,
    ) -> Self {
        Self {
            memory_mapping,
            access_type,
        }
    }
    pub fn memory_mapping(&'a self) -> Option<&'a MemoryMapping<'a>> {
        self.memory_mapping
    }
    pub fn access_type(&self) -> Option<AccessType> {
        self.access_type
    }
    pub fn with_access_type(mut self, access_type: Option<AccessType>) -> Self {
        self.access_type = access_type;
        self
    }

    pub fn map_vm_addr_to_host(&'a self, vm_addr: u64, len: u64) -> ProgramResult {
        if let Some(mm) = self.memory_mapping {
            return mm.map(self.access_type.unwrap_or(AccessType::Load), vm_addr, len);
        }
        ProgramResult::Ok(vm_addr)
    }
}
