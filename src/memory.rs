use std::u32;
use std::ops::Range;
use std::cmp;
use std::fmt;
use std::rc::Rc;
use std::cell::RefCell;
use parity_wasm::elements::ResizableLimits;
use Error;
use module::check_limits;

/// Size of a page of [linear memory][`MemoryInstance`] - 64KiB.
///
/// The size of a memory is always a integer multiple of a page size.
///
/// [`MemoryInstance`]: struct.MemoryInstance.html
pub const LINEAR_MEMORY_PAGE_SIZE: u32 = 65536;

/// Maximal number of pages.
const LINEAR_MEMORY_MAX_PAGES: u32 = 65536;

/// Reference to a memory (See [`MemoryInstance`] for details).
///
/// This reference has a reference-counting semantics.
///
/// [`MemoryInstance`]: struct.MemoryInstance.html
///
#[derive(Clone, Debug)]
pub struct MemoryRef(Rc<MemoryInstance>);

impl ::std::ops::Deref for MemoryRef {
	type Target = MemoryInstance;
	fn deref(&self) -> &MemoryInstance {
		&self.0
	}
}

/// Runtime representation of a linear memory (or `memory` for short).
///
/// A memory is a contiguous, mutable array of raw bytes. Wasm code can load and store values
/// from/to a linear memory at any byte address.
/// A trap occurs if an access is not within the bounds of the current memory size.
///
/// A memory is created with an initial size but can be grown dynamically.
/// The growth can be limited by specifying maximum size.
/// The size of a memory is always a integer multiple of a [page size][`LINEAR_MEMORY_PAGE_SIZE`] - 64KiB.
///
/// At the moment, wasm doesn't provide any way to shrink the memory.
///
/// [`LINEAR_MEMORY_PAGE_SIZE`]: constant.LINEAR_MEMORY_PAGE_SIZE.html
pub struct MemoryInstance {
	/// Memofy limits.
	limits: ResizableLimits,
	/// Linear memory buffer.
	buffer: RefCell<Vec<u8>>,
	/// Maximum buffer size.
	maximum_size: u32,
}

impl fmt::Debug for MemoryInstance {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("MemoryInstance")
			.field("limits", &self.limits)
			.field("buffer.len", &self.buffer.borrow().len())
			.field("maximum_size", &self.maximum_size)
			.finish()
	}
}

struct CheckedRegion<'a, B: 'a> where B: ::std::ops::Deref<Target=Vec<u8>> {
	buffer: &'a B,
	offset: usize,
	size: usize,
}

impl<'a, B: 'a> CheckedRegion<'a, B> where B: ::std::ops::Deref<Target=Vec<u8>> {
	fn range(&self) -> Range<usize> {
		self.offset..self.offset+self.size
	}

	fn slice(&self) -> &[u8] {
		&self.buffer[self.range()]
	}

	fn intersects(&self, other: &Self) -> bool {
		let low = cmp::max(self.offset, other.offset);
		let high = cmp::min(self.offset + self.size, other.offset + other.size);

		low < high
	}
}

impl MemoryInstance {
	/// Allocate a memory instance.
	///
	/// The memory allocated with initial number of pages specified by `initial_pages`.
	/// Minimal possible value for `initial_pages` is 0 and maximum possible is `65536`.
	/// (Since maximum addressible memory is 2<sup>32</sup> = 4GiB = 65536 * [64KiB][`LINEAR_MEMORY_PAGE_SIZE`]).
	///
	/// It is possible to limit maximum number of pages this memory instance can have by specifying
	/// `maximum_page`. If not specified, this memory instance would be able to allocate up to 4GiB.
	///
	/// Allocated memory is always zeroed.
	///
	/// # Errors
	///
	/// Returns `Err` if:
	///
	/// - `initial_pages` is greater than `maximum_pages`
	/// - either `initial_pages` or `maximum_pages` is greater than `65536`.
	///
	/// [`LINEAR_MEMORY_PAGE_SIZE`]: constant.LINEAR_MEMORY_PAGE_SIZE.html
	pub fn alloc(initial_pages: u32, maximum_pages: Option<u32>) -> Result<MemoryRef, Error> {
		let memory = MemoryInstance::new(ResizableLimits::new(initial_pages, maximum_pages))?;
		Ok(MemoryRef(Rc::new(memory)))
	}

	/// Create new linear memory instance.
	fn new(limits: ResizableLimits) -> Result<Self, Error> {
		check_limits(&limits)?;

		let initial_pages = limits.initial();
		let maximum_pages = limits.maximum().unwrap_or(LINEAR_MEMORY_MAX_PAGES);

		if initial_pages > LINEAR_MEMORY_MAX_PAGES {
			return Err(Error::Memory(format!("initial memory size must be at most {} pages", LINEAR_MEMORY_MAX_PAGES)));
		}
		if maximum_pages > LINEAR_MEMORY_MAX_PAGES {
			return Err(Error::Memory(format!("maximum memory size must be at most {} pages", LINEAR_MEMORY_MAX_PAGES)));
		}

		let maximum_size = maximum_pages.saturating_mul(LINEAR_MEMORY_PAGE_SIZE);
		let initial_size = initial_pages.saturating_mul(LINEAR_MEMORY_PAGE_SIZE);

		let memory = MemoryInstance {
			limits: limits,
			buffer: RefCell::new(vec![0; initial_size as usize]),
			maximum_size: maximum_size,
		};

		Ok(memory)
	}

	/// Return linear memory limits.
	pub(crate) fn limits(&self) -> &ResizableLimits {
		&self.limits
	}

	/// Returns number of pages this `MemoryInstance` was created with.
	pub fn initial_pages(&self) -> u32 {
		self.limits.initial()
	}

	/// Returns maximum amount of pages this `MemoryInstance` can grow to.
	///
	/// Returns `None` if there is no limit set.
	/// This means that memory can grow up to 4GiB.
	pub fn maximum_pages(&self) -> Option<u32> {
		self.limits.maximum()
	}

	/// Return linear memory size (in pages).
	pub fn size(&self) -> u32 {
		self.buffer.borrow().len() as u32 / LINEAR_MEMORY_PAGE_SIZE
	}

	/// Copy data from memory at given offset.
	pub fn get(&self, offset: u32, size: usize) -> Result<Vec<u8>, Error> {
		let buffer = self.buffer.borrow();
		let region = self.checked_region(&buffer, offset as usize, size)?;

		Ok(region.slice().to_vec())
	}

	/// Copy data from given offset in the memory into `target` slice.
	///
	/// # Errors
	///
	/// Returns `Err` if the specified region is out of bounds.
	pub fn get_into(&self, offset: u32, target: &mut [u8]) -> Result<(), Error> {
		let buffer = self.buffer.borrow();
		let region = self.checked_region(&buffer, offset as usize, target.len())?;

		target.copy_from_slice(region.slice());

		Ok(())
	}

	/// Copy data in the memory at given offset.
	pub fn set(&self, offset: u32, value: &[u8]) -> Result<(), Error> {
		let mut buffer = self.buffer.borrow_mut();
		let range = self.checked_region(&buffer, offset as usize, value.len())?.range();

		buffer[range].copy_from_slice(value);

		Ok(())
	}

	/// Increases the size of the linear memory by given number of pages.
	/// Returns previous memory size (in pages) if succeeds.
	///
	/// # Errors
	///
	/// Returns `Err` if attempted to allocate more memory than permited by the limit.
	pub fn grow(&self, pages: u32) -> Result<u32, Error> {
		let mut buffer = self.buffer.borrow_mut();
		let old_size = buffer.len() as u32;
		match calculate_memory_size(old_size, pages, self.maximum_size) {
			None => Err(Error::Memory(
				format!(
					"Trying to grow memory by {} pages when already have {}",
					pages,
					old_size / LINEAR_MEMORY_PAGE_SIZE,
				)
			)),
			Some(new_size) => {
				buffer.resize(new_size as usize, 0);
				Ok(old_size / LINEAR_MEMORY_PAGE_SIZE)
			},
		}
	}

	fn checked_region<'a, B>(&self, buffer: &'a B, offset: usize, size: usize) -> Result<CheckedRegion<'a, B>, Error>
		where B: ::std::ops::Deref<Target=Vec<u8>>
	{
		let end = offset.checked_add(size)
			.ok_or_else(|| Error::Memory(format!("trying to access memory block of size {} from offset {}", size, offset)))?;

		if end > buffer.len() {
			return Err(Error::Memory(format!("trying to access region [{}..{}] in memory [0..{}]", offset, end, buffer.len())));
		}

		Ok(CheckedRegion {
			buffer: buffer,
			offset: offset,
			size: size,
		})
	}

	/// Copy contents of one memory region to another.
	///
	/// Semantically equivalent to `memmove`.
	///
	/// # Errors
	///
	/// Returns `Err` if either of specified regions is out of bounds.
	pub fn copy(&self, src_offset: usize, dst_offset: usize, len: usize) -> Result<(), Error> {
		let buffer = self.buffer.borrow_mut();

		let read_region = self.checked_region(&buffer, src_offset, len)?;
		let write_region = self.checked_region(&buffer, dst_offset, len)?;

		unsafe { ::std::ptr::copy(
			buffer[read_region.range()].as_ptr(),
			buffer[write_region.range()].as_ptr() as *mut _,
			len,
		)}

		Ok(())
	}

	/// Copy contents of one memory region to another (non-overlapping version).
	///
	/// Semantically equivalent to `memcpy`.
	/// but returns Error if source overlaping with destination.
	///
	/// # Errors
	///
	/// Returns `Err` if:
	///
	/// - either of specified regions is out of bounds,
	/// - these regions overlaps.
	pub fn copy_nonoverlapping(&self, src_offset: usize, dst_offset: usize, len: usize) -> Result<(), Error> {
		let buffer = self.buffer.borrow_mut();

		let read_region = self.checked_region(&buffer, src_offset, len)?;
		let write_region = self.checked_region(&buffer, dst_offset, len)?;

		if read_region.intersects(&write_region) {
			return Err(Error::Memory(format!("non-overlapping copy is used for overlapping regions")))
		}

		unsafe { ::std::ptr::copy_nonoverlapping(
			buffer[read_region.range()].as_ptr(),
			buffer[write_region.range()].as_ptr() as *mut _,
			len,
		)}

		Ok(())
	}

	/// Fill memory region with a specified value.
	///
	/// Semantically equivalent to `memset`.
	///
	/// # Errors
	///
	/// Returns `Err` if the specified region is out of bounds.
	pub fn clear(&self, offset: usize, new_val: u8, len: usize) -> Result<(), Error> {
		let mut buffer = self.buffer.borrow_mut();

		let range = self.checked_region(&buffer, offset, len)?.range();
		for val in &mut buffer[range] { *val = new_val }
		Ok(())
	}

	/// Fill specified memory region with zeroes.
	///
	/// # Errors
	///
	/// Returns `Err` if the specified region is out of bounds.
	pub fn zero(&self, offset: usize, len: usize) -> Result<(), Error> {
		self.clear(offset, 0, len)
	}
}

fn calculate_memory_size(old_size: u32, additional_pages: u32, maximum_size: u32) -> Option<u32> {
	let size = additional_pages
		.saturating_mul(LINEAR_MEMORY_PAGE_SIZE);
	match size.checked_add(old_size) {
		Some(size) if size <= maximum_size => Some(size),
		_ => None,
	}
}

#[cfg(test)]
mod tests {

	use super::{MemoryInstance, LINEAR_MEMORY_MAX_PAGES};
	use Error;
	use parity_wasm::elements::ResizableLimits;

	#[test]
	fn alloc() {
		let fixtures = &[
			(0, None, true),
			(0, Some(0), true),
			(1, None, true),
			(1, Some(1), true),
			(0, Some(1), true),
			(1, Some(0), false),
			(0, Some(LINEAR_MEMORY_MAX_PAGES), true),
			(LINEAR_MEMORY_MAX_PAGES, Some(LINEAR_MEMORY_MAX_PAGES), true),
			(LINEAR_MEMORY_MAX_PAGES, Some(0), false),
			(LINEAR_MEMORY_MAX_PAGES, None, true),
		];
		for &(initial, maybe_max, expected_ok) in fixtures {
			let result = MemoryInstance::alloc(initial, maybe_max);
			if result.is_ok() != expected_ok {
				panic!(
					"unexpected error, initial={}, max={:?}, expected={}, result={:?}",
					initial,
					maybe_max,
					expected_ok,
					result,
				);
			}
		}
	}

	fn create_memory(initial_content: &[u8]) -> MemoryInstance {
		let mem = MemoryInstance::new(ResizableLimits::new(1, Some(1)))
			.expect("MemoryInstance created successfuly");
		mem.set(0, initial_content).expect("Successful initialize the memory");
		mem
	}

	#[test]
	fn copy_overlaps_1() {
		let mem = create_memory(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		mem.copy(0, 4, 6).expect("Successfully copy the elements");
		let result = mem.get(0, 10).expect("Successfully retrieve the result");
		assert_eq!(result, &[0, 1, 2, 3, 0, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn copy_overlaps_2() {
		let mem = create_memory(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		mem.copy(4, 0, 6).expect("Successfully copy the elements");
		let result = mem.get(0, 10).expect("Successfully retrieve the result");
		assert_eq!(result, &[4, 5, 6, 7, 8, 9, 6, 7, 8, 9]);
	}

	#[test]
	fn copy_nonoverlapping() {
		let mem = create_memory(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		mem.copy_nonoverlapping(0, 10, 10).expect("Successfully copy the elements");
		let result = mem.get(10, 10).expect("Successfully retrieve the result");
		assert_eq!(result, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
	}

	#[test]
	fn copy_nonoverlapping_overlaps_1() {
		let mem = create_memory(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		let result = mem.copy_nonoverlapping(0, 4, 6);
		match result {
			Err(Error::Memory(_)) => {},
			_ => panic!("Expected Error::Memory(_) result, but got {:?}", result),
		}
	}

	#[test]
	fn copy_nonoverlapping_overlaps_2() {
		let mem = create_memory(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		let result = mem.copy_nonoverlapping(4, 0, 6);
		match result {
			Err(Error::Memory(_)) => {},
			_ => panic!("Expected Error::Memory(_), but got {:?}", result),
		}
	}

	#[test]
	fn clear() {
		let mem = create_memory(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		mem.clear(0, 0x4A, 10).expect("To successfully clear the memory");
		let result = mem.get(0, 10).expect("To successfully retrieve the result");
		assert_eq!(result, &[0x4A; 10]);
	}

	#[test]
	fn get_into() {
		let mem = MemoryInstance::new(ResizableLimits::new(1, None)).expect("memory instance creation should not fail");
		mem.set(6, &[13, 17, 129]).expect("memory set should not fail");

		let mut data = [0u8; 2];
		mem.get_into(7, &mut data[..]).expect("get_into should not fail");

		assert_eq!(data, [17, 129]);
	}
}
