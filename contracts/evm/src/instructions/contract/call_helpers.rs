use core::ops::Range;
use revm_interpreter::{
    as_usize_or_fail_ret,
    interpreter::Interpreter,
    pop_ret,
    primitives::{Bytes, U256},
    resize_memory,
};

#[inline]
pub fn get_memory_input_and_out_ranges(
    interpreter: &mut Interpreter,
) -> Option<(Bytes, Range<usize>)> {
    pop_ret!(interpreter, in_offset, in_len, out_offset, out_len, None);

    let in_range = resize_memory(interpreter, in_offset, in_len)?;

    let mut input = Bytes::new();
    if !in_range.is_empty() {
        input = Bytes::copy_from_slice(interpreter.shared_memory.slice_range(in_range));
    }

    let ret_range = resize_memory(interpreter, out_offset, out_len)?;
    Some((input, ret_range))
}

/// Resize memory and return range of memory.
/// If `len` is 0 dont touch memory and return `usize::MAX` as offset and 0 as length.
#[inline]
pub fn resize_memory(
    interpreter: &mut Interpreter,
    offset: U256,
    len: U256,
) -> Option<Range<usize>> {
    let len = as_usize_or_fail_ret!(interpreter, len, None);
    let offset = if len != 0 {
        let offset = as_usize_or_fail_ret!(interpreter, offset, None);
        resize_memory!(interpreter, offset, len, None);
        offset
    } else {
        usize::MAX //unrealistic value so we are sure it is not used
    };
    Some(offset..offset + len)
}
