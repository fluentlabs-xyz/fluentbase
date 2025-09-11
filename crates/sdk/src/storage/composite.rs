use crate::{
    storage::{StorageDescriptor, StorageLayout},
    U256,
};
use core::marker::PhantomData;

// TODO(d1r1): double-check if we actually need custom Clone, Copy impls here
#[derive(Debug, PartialEq, Eq)]
pub struct Composite<T> {
    base_slot: U256,
    _phantom: PhantomData<T>,
}

impl<T> Clone for Composite<T> {
    fn clone(&self) -> Self {
        Self {
            base_slot: self.base_slot,
            _phantom: PhantomData,
        }
    }
}
impl<T> Copy for Composite<T> {}

impl<T> Composite<T> {
    pub const fn new(base_slot: U256) -> Self {
        Self {
            base_slot,
            _phantom: PhantomData,
        }
    }
}

impl<T> StorageDescriptor for Composite<T> {
    fn new(slot: U256, offset: u8) -> Self {
        debug_assert_eq!(offset, 0, "Composite types always start at slot boundary");
        Self::new(slot)
    }

    fn slot(&self) -> U256 {
        self.base_slot
    }

    fn offset(&self) -> u8 {
        0
    }
}

// ===== CompositeStorage trait =====

/// Trait for composite storage types that can be wrapped in Composite<T>
pub trait CompositeStorage: Sized {
    /// Number of slots required for this composite type
    const REQUIRED_SLOTS: usize;

    /// Create instance from base slot
    fn from_slot(base_slot: U256) -> Self;
}

// Implement StorageLayout for Composite<T>
impl<T: CompositeStorage> StorageLayout for Composite<T> {
    type Descriptor = Self;
    type Accessor = T;

    const REQUIRED_SLOTS: usize = T::REQUIRED_SLOTS;
    const ENCODED_SIZE: usize = T::REQUIRED_SLOTS * 32;

    fn access(descriptor: Self::Descriptor) -> Self::Accessor {
        T::from_slot(descriptor.base_slot)
    }
}
