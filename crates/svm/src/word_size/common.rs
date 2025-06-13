use solana_rbpf::{
    error::ProgramResult,
    memory_region::{AccessType, MemoryMapping},
};

pub const FIXED_MACHINE_WORD_BYTE_SIZE: usize = crate::typ_size!(u64);
pub const FIXED_PTR_BYTE_SIZE: usize = FIXED_MACHINE_WORD_BYTE_SIZE;
pub const FAT_PTR64_ELEM_BYTE_SIZE: usize = FIXED_MACHINE_WORD_BYTE_SIZE;
pub const SLICE_FAT_PTR64_SIZE_BYTES: usize = FAT_PTR64_ELEM_BYTE_SIZE * 2;
pub const STABLE_VEC_FAT_PTR64_BYTE_SIZE: usize = FAT_PTR64_ELEM_BYTE_SIZE * 3;

#[inline(always)]
fn validate_typecast<T: Clone>(data: &[u8]) {
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
        MemoryMappingHelper::new(None)
    }
}

impl<'a> MemoryMappingHelper<'a> {
    pub fn new(memory_mapping: Option<&'a MemoryMapping<'a>>) -> Self {
        Self {
            memory_mapping,
            access_type: None,
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

impl<'a: 'b, 'b> From<&'a MemoryMapping<'b>> for MemoryMappingHelper<'b> {
    fn from(value: &'a MemoryMapping<'b>) -> Self {
        Self::new(Some(value))
    }
}

impl<'a: 'b, 'b> From<&'a mut MemoryMapping<'b>> for MemoryMappingHelper<'b> {
    fn from(value: &'a mut MemoryMapping<'b>) -> Self {
        Self::new(Some(value))
    }
}
