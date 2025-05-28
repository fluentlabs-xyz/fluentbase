use crate::{
    gas,
    gas::{memory_gas, Gas},
};
use alloc::vec::Vec;
use core::{cmp::min, ops::Range};
use fluentbase_sdk::{B256, U256};

/// A sequential memory shared between calls, which uses
/// a `Vec` for internal representation.
/// A [SharedMemory] instance should always be obtained using
/// the `new` static method to ensure memory safety.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SharedMemory {
    /// The underlying buffer.
    buffer: Vec<u8>,
    /// Memory checkpoints for each depth.
    /// Invariant: these are always in bounds of `data`.
    checkpoints: Vec<usize>,
    /// Invariant: equals `self.checkpoints.last()`
    last_checkpoint: usize,
}

impl Default for SharedMemory {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl SharedMemory {
    /// Creates a new memory instance that can be shared between calls.
    ///
    /// The default initial capacity is 4KiB.
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(4 * 1024) // from evmone
    }

    /// Creates a new memory instance that can be shared between calls with the given `capacity`.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            checkpoints: Vec::with_capacity(32),
            last_checkpoint: 0,
        }
    }

    /// Prepares the shared memory for a new context.
    #[inline]
    pub fn new_context(&mut self) {
        let new_checkpoint = self.buffer.len();
        self.checkpoints.push(new_checkpoint);
        self.last_checkpoint = new_checkpoint;
    }

    /// Prepares the shared memory for returning to the previous context.
    #[inline]
    pub fn free_context(&mut self) {
        if let Some(old_checkpoint) = self.checkpoints.pop() {
            self.last_checkpoint = self.checkpoints.last().cloned().unwrap_or_default();
            // SAFETY: buffer length is less than or equal `old_checkpoint`
            unsafe { self.buffer.set_len(old_checkpoint) };
        }
    }

    /// Returns the length of the current memory range.
    #[inline]
    pub fn len(&self) -> usize {
        self.buffer.len() - self.last_checkpoint
    }

    /// Returns the gas cost for the current memory expansion.
    #[inline]
    pub fn current_expansion_cost(&self) -> u64 {
        gas::memory_gas_for_len(self.len())
    }

    /// Resizes the memory in-place so that `len` is equal to `new_len`.
    #[inline]
    pub fn resize(&mut self, new_size: usize) {
        self.buffer.resize(self.last_checkpoint + new_size, 0);
    }

    /// Returns a byte slice of the memory region at the given offset.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn slice(&self, offset: usize, size: usize) -> &[u8] {
        self.slice_range(offset..offset + size)
    }

    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn try_slice(&self, offset: usize, size: usize) -> Option<&[u8]> {
        self.context_memory().get(offset..offset.checked_add(size)?)
    }

    /// Returns a byte slice of the memory region at the given offset.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn slice_range(&self, range @ Range { start, end }: Range<usize>) -> &[u8] {
        match self.context_memory().get(range) {
            Some(slice) => slice,
            None => unreachable!("slice OOB: {start}..{end}; len: {}", self.len()),
        }
    }

    /// Returns a byte slice of the memory region at the given offset.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn slice_mut(&mut self, offset: usize, size: usize) -> &mut [u8] {
        let end = offset + size;
        match self.context_memory_mut().get_mut(offset..end) {
            Some(slice) => slice,
            None => unreachable!("slice OOB: {offset}..{end}"),
        }
    }

    /// Returns a 32-byte slice of the memory region at the given offset.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    pub fn get_word(&self, offset: usize) -> B256 {
        self.slice(offset, 32).try_into().unwrap()
    }

    /// Returns a U256 of the memory region at the given offset.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    pub fn get_u256(&self, offset: usize) -> U256 {
        self.get_word(offset).into()
    }

    /// Sets the `byte` at the given `index`.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn set_byte(&mut self, offset: usize, byte: u8) {
        self.set(offset, &[byte]);
    }

    /// Sets the given U256 `value` to the memory region at the given `offset`.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn set_u256(&mut self, offset: usize, value: U256) {
        self.set(offset, &value.to_be_bytes::<32>());
    }

    /// Set memory region at given `offset`.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn set(&mut self, offset: usize, value: &[u8]) {
        if !value.is_empty() {
            self.slice_mut(offset, value.len()).copy_from_slice(value);
        }
    }

    /// Set memory from data. Our memory offset+len is expected to be correct but we
    /// are doing bound checks on data/data_offset/len and zeroing parts that is not copied.
    ///
    /// # Panics
    ///
    /// Panics if memory is out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn set_data(&mut self, memory_offset: usize, data_offset: usize, len: usize, data: &[u8]) {
        if data_offset >= data.len() {
            // nullify all memory slots
            self.slice_mut(memory_offset, len).fill(0);
            return;
        }
        let data_end = min(data_offset + len, data.len());
        let data_len = data_end - data_offset;
        debug_assert!(data_offset < data.len() && data_end <= data.len());
        let data = unsafe { data.get_unchecked(data_offset..data_end) };
        self.slice_mut(memory_offset, data_len)
            .copy_from_slice(data);

        // nullify rest of memory slots
        // SAFETY: Memory is assumed to be valid, and it is commented where this assumption is made.
        self.slice_mut(memory_offset + data_len, len - data_len)
            .fill(0);
    }

    /// Copies elements from one part of the memory to another part of itself.
    ///
    /// # Panics
    ///
    /// Panics on out of bounds.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn copy(&mut self, dst: usize, src: usize, len: usize) {
        self.context_memory_mut().copy_within(src..src + len, dst);
    }

    /// Returns a reference to the memory of the current context, the active memory.
    #[inline]
    pub fn context_memory(&self) -> &[u8] {
        // SAFETY: access bounded by buffer length
        unsafe {
            self.buffer
                .get_unchecked(self.last_checkpoint..self.buffer.len())
        }
    }

    /// Returns a mutable reference to the memory of the current context.
    #[inline]
    pub fn context_memory_mut(&mut self) -> &mut [u8] {
        let buf_len = self.buffer.len();
        // SAFETY: access bounded by buffer length
        unsafe { self.buffer.get_unchecked_mut(self.last_checkpoint..buf_len) }
    }
}

/// Returns number of words what would fit to provided number of bytes,
/// i.e. it rounds up the number bytes to number of words.
#[inline]
pub const fn num_words(len: u64) -> u64 {
    len.saturating_add(31) / 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_num_words() {
        assert_eq!(num_words(0), 0);
        assert_eq!(num_words(1), 1);
        assert_eq!(num_words(31), 1);
        assert_eq!(num_words(32), 1);
        assert_eq!(num_words(33), 2);
        assert_eq!(num_words(63), 2);
        assert_eq!(num_words(64), 2);
        assert_eq!(num_words(65), 3);
        assert_eq!(num_words(u64::MAX), u64::MAX / 32);
    }

    #[test]
    fn new_free_context() {
        let mut memory = SharedMemory::new();
        memory.new_context();

        assert_eq!(memory.buffer.len(), 0);
        assert_eq!(memory.checkpoints.len(), 1);
        assert_eq!(memory.last_checkpoint, 0);

        unsafe { memory.buffer.set_len(32) };
        assert_eq!(memory.len(), 32);
        memory.new_context();

        assert_eq!(memory.buffer.len(), 32);
        assert_eq!(memory.checkpoints.len(), 2);
        assert_eq!(memory.last_checkpoint, 32);
        assert_eq!(memory.len(), 0);

        unsafe { memory.buffer.set_len(96) };
        assert_eq!(memory.len(), 64);
        memory.new_context();

        assert_eq!(memory.buffer.len(), 96);
        assert_eq!(memory.checkpoints.len(), 3);
        assert_eq!(memory.last_checkpoint, 96);
        assert_eq!(memory.len(), 0);

        // free contexts
        memory.free_context();
        assert_eq!(memory.buffer.len(), 96);
        assert_eq!(memory.checkpoints.len(), 2);
        assert_eq!(memory.last_checkpoint, 32);
        assert_eq!(memory.len(), 64);

        memory.free_context();
        assert_eq!(memory.buffer.len(), 32);
        assert_eq!(memory.checkpoints.len(), 1);
        assert_eq!(memory.last_checkpoint, 0);
        assert_eq!(memory.len(), 32);

        memory.free_context();
        assert_eq!(memory.buffer.len(), 0);
        assert_eq!(memory.checkpoints.len(), 0);
        assert_eq!(memory.last_checkpoint, 0);
        assert_eq!(memory.len(), 0);
    }

    #[test]
    fn resize() {
        let mut memory = SharedMemory::new();
        memory.new_context();

        memory.resize(32);
        assert_eq!(memory.buffer.len(), 32);
        assert_eq!(memory.len(), 32);
        assert_eq!(memory.buffer.get(0..32), Some(&[0_u8; 32] as &[u8]));

        memory.new_context();
        memory.resize(96);
        assert_eq!(memory.buffer.len(), 128);
        assert_eq!(memory.len(), 96);
        assert_eq!(memory.buffer.get(32..128), Some(&[0_u8; 96] as &[u8]));

        memory.free_context();
        memory.resize(64);
        assert_eq!(memory.buffer.len(), 64);
        assert_eq!(memory.len(), 64);
        assert_eq!(memory.buffer.get(0..64), Some(&[0_u8; 64] as &[u8]));
    }
}

/// Resize the memory to the new size. Returns whether the gas was enough to resize the memory.
#[inline(never)]
#[cold]
#[must_use]
pub fn resize_memory(memory: &mut SharedMemory, gas: &mut Gas, new_size: usize) -> bool {
    let new_words = num_words(new_size as u64);
    let new_cost = memory_gas(new_words);
    let current_cost = memory.current_expansion_cost();
    let cost = new_cost - current_cost;
    let success = gas.record_cost(cost);
    if success {
        memory.resize((new_words as usize) * 32);
    }
    success
}
