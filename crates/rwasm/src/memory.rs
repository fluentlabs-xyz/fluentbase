use revm_interpreter::SharedMemory;
use rwasm::{
    core::{Pages, TrapCode},
    engine::executor::EntityGrowError,
    errors::MemoryError,
    store::ResourceLimiterRef,
    MemoryType,
};

pub struct GlobalMemory {
    pub shared_memory: SharedMemory,
    pub memory_type: MemoryType,
    pub current_pages: Pages,
}

impl GlobalMemory {
    pub fn new(
        mut shared_memory: SharedMemory,
        memory_type: MemoryType,
        limiter: &mut ResourceLimiterRef<'_>,
    ) -> Result<Self, MemoryError> {
        let initial_pages = memory_type.initial_pages();
        let initial_len = initial_pages.to_bytes();
        let maximum_pages = memory_type.maximum_pages().unwrap_or_else(Pages::max);
        let maximum_len = maximum_pages.to_bytes();
        if let Some(limiter) = limiter.as_resource_limiter() {
            if !limiter.memory_growing(0, initial_len.unwrap_or(usize::MAX), maximum_len)? {
                return Err(MemoryError::OutOfBoundsAllocation);
            }
        }
        if let Some(initial_len) = initial_len {
            shared_memory.resize(initial_len);
            let memory = Self {
                shared_memory,
                memory_type,
                current_pages: initial_pages,
            };
            Ok(memory)
        } else {
            let err = MemoryError::OutOfBoundsAllocation;
            if let Some(limiter) = limiter.as_resource_limiter() {
                limiter.memory_grow_failed(&err)
            }
            Err(err)
        }
    }

    /// Returns the memory type of the linear memory.
    pub fn ty(&self) -> MemoryType {
        self.memory_type
    }

    /// Returns the amount of pages in use by the linear memory.
    pub fn current_pages(&self) -> Pages {
        self.current_pages
    }

    /// Grows the linear memory by the given amount of new pages.
    ///
    /// Returns the amount of pages before the operation upon success.
    ///
    /// # Errors
    ///
    /// If the linear memory would grow beyond its maximum limit after
    /// the grow operation.
    pub fn grow(
        &mut self,
        additional: Pages,
        limiter: &mut ResourceLimiterRef<'_>,
    ) -> Result<Pages, EntityGrowError> {
        let current_pages = self.current_pages();
        if additional == Pages::from(0) {
            // Nothing to do in this case. Bail out early.
            return Ok(current_pages);
        }

        let maximum_pages = self.ty().maximum_pages().unwrap_or_else(Pages::max);
        let desired_pages = current_pages.checked_add(additional);

        // ResourceLimiter gets the first look at the request.
        if let Some(limiter) = limiter.as_resource_limiter() {
            let current_size = current_pages.to_bytes().unwrap_or(usize::MAX);
            let desired_size = desired_pages
                .unwrap_or_else(Pages::max)
                .to_bytes()
                .unwrap_or(usize::MAX);
            let maximum_size = maximum_pages.to_bytes();
            match limiter.memory_growing(current_size, desired_size, maximum_size) {
                Ok(true) => (),
                Ok(false) => return Err(EntityGrowError::InvalidGrow),
                Err(_) => return Err(EntityGrowError::TrapCode(TrapCode::GrowthOperationLimited)),
            }
        }

        let mut ret: Result<Pages, EntityGrowError> = Err(EntityGrowError::InvalidGrow);

        if let Some(new_pages) = desired_pages {
            if new_pages <= maximum_pages {
                if let Some(new_size) = new_pages.to_bytes() {
                    // At this point, it is okay to grow the underlying virtual memory
                    // by the given number of additional pages.
                    assert!(new_size >= self.shared_memory.len());
                    self.shared_memory.resize(new_size);
                    self.current_pages = new_pages;
                    ret = Ok(current_pages)
                }
            }
        }

        // If there was an error, ResourceLimiter gets to see.
        if ret.is_err() {
            if let Some(limiter) = limiter.as_resource_limiter() {
                limiter.memory_grow_failed(&MemoryError::OutOfBoundsGrowth)
            }
        }

        ret
    }

    /// Returns a shared slice to the bytes underlying to the byte buffer.
    pub fn data(&self) -> &[u8] {
        self.shared_memory.context_memory()
    }

    /// Returns an exclusive slice to the bytes underlying to the byte buffer.
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.shared_memory.context_memory_mut()
    }

    /// Reads `n` bytes from `memory[offset..offset+n]` into `buffer`
    /// where `n` is the length of `buffer`.
    ///
    /// # Errors
    ///
    /// If this operation accesses out of bounds linear memory.
    pub fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<(), MemoryError> {
        let len_buffer = buffer.len();
        let slice = self
            .data()
            .get(offset..(offset + len_buffer))
            .ok_or(MemoryError::OutOfBoundsAccess)?;
        buffer.copy_from_slice(slice);
        Ok(())
    }

    /// Writes `n` bytes to `memory[offset..offset+n]` from `buffer`
    /// where `n` if the length of `buffer`.
    ///
    /// # Errors
    ///
    /// If this operation accesses out of bounds linear memory.
    pub fn write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), MemoryError> {
        let len_buffer = buffer.len();
        let slice = self
            .data_mut()
            .get_mut(offset..(offset + len_buffer))
            .ok_or(MemoryError::OutOfBoundsAccess)?;
        slice.copy_from_slice(buffer);
        Ok(())
    }
}
