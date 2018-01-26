use std::u32;
use std::fmt;
use std::cell::RefCell;
use std::rc::Rc;
use parity_wasm::elements::ResizableLimits;
use Error;
use func::FuncRef;
use module::check_limits;

/// Reference to a table (See [`TableInstance`] for details).
///
/// This reference has a reference-counting semantics.
///
/// [`TableInstance`]: struct.TableInstance.html
///
#[derive(Clone, Debug)]
pub struct TableRef(Rc<TableInstance>);

impl ::std::ops::Deref for TableRef {
	type Target = TableInstance;
	fn deref(&self) -> &TableInstance {
		&self.0
	}
}

/// Runtime representation of a table.
///
/// A table is a array of untyped functions. It allows wasm code to call functions
/// indirectly through a dynamic index into a table. For example, this allows emulating function
/// pointers by way of table indices.
///
/// Table is created with an initial size but can be grown dynamically via [`grow`] method.
/// Growth can be limited by an optional maximum size.
///
/// In future, a table might be extended to be able to hold not only functions but different types.
///
/// [`grow`]: #method.grow
///
pub struct TableInstance {
	/// Table limits.
	limits: ResizableLimits,
	/// Table memory buffer.
	buffer: RefCell<Vec<Option<FuncRef>>>,
}

impl fmt::Debug for TableInstance {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("TableInstance")
			.field("limits", &self.limits)
			.field("buffer.len", &self.buffer.borrow().len())
			.finish()
	}
}

impl TableInstance {

	pub fn alloc(initial_size: u32, maximum_size: Option<u32>) -> Result<TableRef, Error> {
		let table = TableInstance::new(ResizableLimits::new(initial_size, maximum_size))?;
		Ok(TableRef(Rc::new(table)))
	}

	fn new(limits: ResizableLimits) -> Result<TableInstance, Error> {
		check_limits(&limits)?;
		Ok(TableInstance {
			buffer: RefCell::new(vec![None; limits.initial() as usize]),
			limits: limits,
		})
	}

	/// Return table limits.
	pub(crate) fn limits(&self) -> &ResizableLimits {
		&self.limits
	}

	pub fn initial_size(&self) -> u32 {
		self.limits.initial()
	}

	pub fn maximum_size(&self) -> Option<u32> {
		self.limits.maximum()
	}

	pub fn current_size(&self) -> u32 {
		self.buffer.borrow().len() as u32
	}

	pub fn grow(&self, by: u32) -> Result<(), Error> {
		let mut buffer = self.buffer.borrow_mut();
		let maximum_size = self.maximum_size().unwrap_or(u32::MAX);
		let new_size = self.current_size().checked_add(by)
			.and_then(|new_size| {
				if maximum_size < new_size {
					None
				} else {
					Some(new_size)
				}
			})
			.ok_or_else(||
				Error::Table(format!(
					"Trying to grow table by {} items when there are already {} items",
					by,
					self.current_size(),
				))
			)?;
		buffer.resize(new_size as usize, None);
		Ok(())
	}

	/// Get the specific value in the table
	pub fn get(&self, offset: u32) -> Result<FuncRef, Error> {
		let buffer = self.buffer.borrow();
		let buffer_len = buffer.len();
		let table_elem = buffer.get(offset as usize).cloned().ok_or_else(||
			Error::Table(format!(
				"trying to read table item with index {} when there are only {} items",
				offset,
				buffer_len
			)),
		)?;
		Ok(table_elem.ok_or_else(|| Error::Table(format!(
			"trying to read uninitialized element on index {}",
			offset
		)))?)
	}

	/// Set the table element to the specified function.
	pub fn set(&self, offset: u32, value: Option<FuncRef>) -> Result<(), Error> {
		let mut buffer = self.buffer.borrow_mut();
		let buffer_len = buffer.len();
		let table_elem = buffer.get_mut(offset as usize).ok_or_else(||
			Error::Table(format!(
				"trying to update table item with index {} when there are only {} items",
				offset,
				buffer_len
			))
		)?;
		*table_elem = value;
		Ok(())
	}
}
