use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use nan_preserving_float::{F32, F64};
use std::io;
use std::mem::transmute;
use std::{f32, i32, i64, u32, u64};
use TrapKind;

#[derive(Debug)]
pub enum Error {
	InvalidLittleEndianBuffer,
}

/// Runtime representation of a value.
///
/// Wasm code manipulate values of the four basic value types:
/// integers and floating-point (IEEE 754-2008) data of 32 or 64 bit width each, respectively.
///
/// There is no distinction between signed and unsigned integer types. Instead, integers are
/// interpreted by respective operations as either unsigned or signed in two’s complement representation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RuntimeValue {
	/// Value of 32-bit signed or unsigned integer.
	I32(i32),
	/// Value of 64-bit signed or unsigned integer.
	I64(i64),
	/// Value of 32-bit IEEE 754-2008 floating point number.
	F32(F32),
	/// Value of 64-bit IEEE 754-2008 floating point number.
	F64(F64),
}

/// Trait for creating value from a [`RuntimeValue`].
///
/// Typically each implementation can create a value from the specific type.
/// For example, values of type `bool` or `u32` are both represented by [`I32`] and `f64` values are represented by
/// [`F64`].
///
/// [`I32`]: enum.RuntimeValue.html#variant.I32
/// [`F64`]: enum.RuntimeValue.html#variant.F64
/// [`RuntimeValue`]: enum.RuntimeValue.html
pub trait FromRuntimeValue where Self: Sized {
	/// Create a value of type `Self` from a given [`RuntimeValue`].
	///
	/// Returns `None` if the [`RuntimeValue`] is of type different than
	/// expected by the conversion in question.
	///
	/// [`RuntimeValue`]: enum.RuntimeValue.html
	fn from_runtime_value(val: RuntimeValue) -> Option<Self>;
}

/// Convert one type to another by wrapping.
pub trait WrapInto<T> {
	/// Convert one type to another by wrapping.
	fn wrap_into(self) -> T;
}

/// Convert one type to another by rounding to the nearest integer towards zero.
pub trait TryTruncateInto<T, E> {
	/// Convert one type to another by rounding to the nearest integer towards zero.
	fn try_truncate_into(self) -> Result<T, E>;
}

/// Convert one type to another by extending with leading zeroes.
pub trait ExtendInto<T> {
	/// Convert one type to another by extending with leading zeroes.
	fn extend_into(self) -> T;
}

/// Reinterprets the bits of a value of one type as another type.
pub trait TransmuteInto<T> {
	/// Reinterprets the bits of a value of one type as another type.
	fn transmute_into(self) -> T;
}

/// Convert from and to little endian.
pub trait LittleEndianConvert where Self: Sized {
	/// Convert to little endian buffer.
	fn into_little_endian(self) -> Vec<u8>;
	/// Convert from little endian buffer.
	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error>;
}

/// Arithmetic operations.
pub trait ArithmeticOps<T> {
	/// Add two values.
	fn add(self, other: T) -> T;
	/// Subtract two values.
	fn sub(self, other: T) -> T;
	/// Multiply two values.
	fn mul(self, other: T) -> T;
	/// Divide two values.
	fn div(self, other: T) -> Result<T, TrapKind>;
}

/// Integer value.
pub trait Integer<T>: ArithmeticOps<T> {
	/// Counts leading zeros in the bitwise representation of the value.
	fn leading_zeros(self) -> T;
	/// Counts trailing zeros in the bitwise representation of the value.
	fn trailing_zeros(self) -> T;
	/// Counts 1-bits in the bitwise representation of the value.
	fn count_ones(self) -> T;
	/// Get left bit rotation result.
	fn rotl(self, other: T) -> T;
	/// Get right bit rotation result.
	fn rotr(self, other: T) -> T;
	/// Get division remainder.
	fn rem(self, other: T) -> Result<T, TrapKind>;
}

/// Float-point value.
pub trait Float<T>: ArithmeticOps<T> {
	/// Get absolute value.
	fn abs(self) -> T;
	/// Returns the largest integer less than or equal to a number.
	fn floor(self) -> T;
	/// Returns the smallest integer greater than or equal to a number.
	fn ceil(self) -> T;
	/// Returns the integer part of a number.
	fn trunc(self) -> T;
	/// Returns the nearest integer to a number. Round half-way cases away from 0.0.
	fn round(self) -> T;
	/// Returns the nearest integer to a number. Ties are round to even number.
	fn nearest(self) -> T;
	/// Takes the square root of a number.
	fn sqrt(self) -> T;
	/// Returns the minimum of the two numbers.
	fn min(self, other: T) -> T;
	/// Returns the maximum of the two numbers.
	fn max(self, other: T) -> T;
	/// Sets sign of this value to the sign of other value.
	fn copysign(self, other: T) -> T;
}

impl RuntimeValue {
	/// Creates new default value of given type.
	pub fn default(value_type: ::types::ValueType) -> Self {
		match value_type {
			::types::ValueType::I32 => RuntimeValue::I32(0),
			::types::ValueType::I64 => RuntimeValue::I64(0),
			::types::ValueType::F32 => RuntimeValue::F32(0f32.into()),
			::types::ValueType::F64 => RuntimeValue::F64(0f64.into()),
		}
	}

	/// Creates new value by interpreting passed u32 as f32.
	pub fn decode_f32(val: u32) -> Self {
		RuntimeValue::F32(F32::from_bits(val))
	}

	/// Creates new value by interpreting passed u64 as f64.
	pub fn decode_f64(val: u64) -> Self {
		RuntimeValue::F64(F64::from_bits(val))
	}

	/// Get variable type for this value.
	pub fn value_type(&self) -> ::types::ValueType {
		match *self {
			RuntimeValue::I32(_) => ::types::ValueType::I32,
			RuntimeValue::I64(_) => ::types::ValueType::I64,
			RuntimeValue::F32(_) => ::types::ValueType::F32,
			RuntimeValue::F64(_) => ::types::ValueType::F64,
		}
	}

	/// Returns `T` if this particular [`RuntimeValue`] contains
	/// appropriate type.
	///
	/// See [`FromRuntimeValue`] for details.
	///
	/// [`FromRuntimeValue`]: trait.FromRuntimeValue.html
	/// [`RuntimeValue`]: enum.RuntimeValue.html
	pub fn try_into<T: FromRuntimeValue>(self) -> Option<T> {
		FromRuntimeValue::from_runtime_value(self)
	}
}

impl From<i8> for RuntimeValue {
	fn from(val: i8) -> Self {
		RuntimeValue::I32(val as i32)
	}
}

impl From<i16> for RuntimeValue {
	fn from(val: i16) -> Self {
		RuntimeValue::I32(val as i32)
	}
}

impl From<i32> for RuntimeValue {
	fn from(val: i32) -> Self {
		RuntimeValue::I32(val)
	}
}

impl From<i64> for RuntimeValue {
	fn from(val: i64) -> Self {
		RuntimeValue::I64(val)
	}
}

impl From<u8> for RuntimeValue {
	fn from(val: u8) -> Self {
		RuntimeValue::I32(val as i32)
	}
}

impl From<u16> for RuntimeValue {
	fn from(val: u16) -> Self {
		RuntimeValue::I32(val as i32)
	}
}

impl From<u32> for RuntimeValue {
	fn from(val: u32) -> Self {
		RuntimeValue::I32(val.transmute_into())
	}
}

impl From<u64> for RuntimeValue {
	fn from(val: u64) -> Self {
		RuntimeValue::I64(val.transmute_into())
	}
}

impl From<F32> for RuntimeValue {
	fn from(val: F32) -> Self {
		RuntimeValue::F32(val)
	}
}

impl From<F64> for RuntimeValue {
	fn from(val: F64) -> Self {
		RuntimeValue::F64(val)
	}
}

macro_rules! impl_from_runtime_value {
	($expected_rt_ty: ident, $into: ty) => {
		impl FromRuntimeValue for $into {
			fn from_runtime_value(val: RuntimeValue) -> Option<Self> {
				match val {
					RuntimeValue::$expected_rt_ty(val) => Some(val.transmute_into()),
					_ => None,
				}
			}
		}
	};
}

/// This conversion assumes that boolean values are represented by
/// [`I32`] type.
///
/// [`I32`]: enum.RuntimeValue.html#variant.I32
impl FromRuntimeValue for bool {
	fn from_runtime_value(val: RuntimeValue) -> Option<Self> {
		match val {
			RuntimeValue::I32(val) => Some(val != 0),
			_ => None,
		}
	}
}

///  This conversion assumes that `i8` is represented as an [`I32`].
/// 
/// [`I32`]: enum.RuntimeValue.html#variant.I32
impl FromRuntimeValue for i8 {
	fn from_runtime_value(val: RuntimeValue) -> Option<Self> {
		let min = i8::min_value() as i32;
		let max = i8::max_value() as i32;
		match val {
			RuntimeValue::I32(val) if min <= val && val <= max => Some(val as i8),
			_ => None,
		}
	}
}

///  This conversion assumes that `i16` is represented as an [`I32`].
/// 
/// [`I32`]: enum.RuntimeValue.html#variant.I32
impl FromRuntimeValue for i16 {
	fn from_runtime_value(val: RuntimeValue) -> Option<Self> {
		let min = i16::min_value() as i32;
		let max = i16::max_value() as i32;
		match val {
			RuntimeValue::I32(val) if min <= val && val <= max => Some(val as i16),
			_ => None,
		}
	}
}

///  This conversion assumes that `u8` is represented as an [`I32`].
/// 
/// [`I32`]: enum.RuntimeValue.html#variant.I32
impl FromRuntimeValue for u8 {
	fn from_runtime_value(val: RuntimeValue) -> Option<Self> {
		let min = u8::min_value() as i32;
		let max = u8::max_value() as i32;
		match val {
			RuntimeValue::I32(val) if min <= val && val <= max => Some(val as u8),
			_ => None,
		}
	}
}

///  This conversion assumes that `u16` is represented as an [`I32`].
/// 
/// [`I32`]: enum.RuntimeValue.html#variant.I32
impl FromRuntimeValue for u16 {
	fn from_runtime_value(val: RuntimeValue) -> Option<Self> {
		let min = u16::min_value() as i32;
		let max = u16::max_value() as i32;
		match val {
			RuntimeValue::I32(val) if min <= val && val <= max => Some(val as u16),
			_ => None,
		}
	}
}

impl_from_runtime_value!(I32, i32);
impl_from_runtime_value!(I64, i64);
impl_from_runtime_value!(F32, F32);
impl_from_runtime_value!(F64, F64);
impl_from_runtime_value!(I32, u32);
impl_from_runtime_value!(I64, u64);

macro_rules! impl_wrap_into {
	($from:ident, $into:ident) => {
		impl WrapInto<$into> for $from {
			fn wrap_into(self) -> $into {
				self as $into
			}
		}
	};
	($from:ident, $intermediate:ident, $into:ident) => {
		impl WrapInto<$into> for $from {
			fn wrap_into(self) -> $into {
				$into::from(self as $intermediate)
			}
		}
	};
}

impl_wrap_into!(i32, i8);
impl_wrap_into!(i32, i16);
impl_wrap_into!(i64, i8);
impl_wrap_into!(i64, i16);
impl_wrap_into!(i64, i32);
impl_wrap_into!(i64, f32, F32);
impl_wrap_into!(u64, f32, F32);
// Casting from an f64 to an f32 will produce the closest possible value (rounding strategy unspecified)
// NOTE: currently this will cause Undefined Behavior if the value is finite but larger or smaller than the
// largest or smallest finite value representable by f32. This is a bug and will be fixed.
impl_wrap_into!(f64, f32);

impl WrapInto<F32> for F64 {
	fn wrap_into(self) -> F32 {
		(f64::from(self) as f32).into()
	}
}

macro_rules! impl_try_truncate_into {
	($from: ident, $into: ident) => {
		impl TryTruncateInto<$into, TrapKind> for $from {
			fn try_truncate_into(self) -> Result<$into, TrapKind> {
				// Casting from a float to an integer will round the float towards zero
				// NOTE: currently this will cause Undefined Behavior if the rounded value cannot be represented by the
				// target integer type. This includes Inf and NaN. This is a bug and will be fixed.
				if self.is_nan() || self.is_infinite() {
					return Err(TrapKind::InvalidConversionToInt);
				}

				// range check
				let result = self as $into;
				if result as $from != self.trunc() {
					return Err(TrapKind::InvalidConversionToInt);
				}

				Ok(self as $into)
			}
		}
	};
	($from:ident, $intermediate:ident, $into:ident) => {
		impl TryTruncateInto<$into, TrapKind> for $from {
			fn try_truncate_into(self) -> Result<$into, TrapKind> {
				$intermediate::from(self).try_truncate_into()
			}
		}
	};
}

impl_try_truncate_into!(f32, i32);
impl_try_truncate_into!(f32, i64);
impl_try_truncate_into!(f64, i32);
impl_try_truncate_into!(f64, i64);
impl_try_truncate_into!(f32, u32);
impl_try_truncate_into!(f32, u64);
impl_try_truncate_into!(f64, u32);
impl_try_truncate_into!(f64, u64);
impl_try_truncate_into!(F32, f32, i32);
impl_try_truncate_into!(F32, f32, i64);
impl_try_truncate_into!(F64, f64, i32);
impl_try_truncate_into!(F64, f64, i64);
impl_try_truncate_into!(F32, f32, u32);
impl_try_truncate_into!(F32, f32, u64);
impl_try_truncate_into!(F64, f64, u32);
impl_try_truncate_into!(F64, f64, u64);

macro_rules! impl_extend_into {
	($from:ident, $into:ident) => {
		impl ExtendInto<$into> for $from {
			fn extend_into(self) -> $into {
				self as $into
			}
		}
	};
	($from:ident, $intermediate:ident, $into:ident) => {
		impl ExtendInto<$into> for $from {
			fn extend_into(self) -> $into {
				$into::from(self as $intermediate)
			}
		}
	};
}

impl_extend_into!(i8, i32);
impl_extend_into!(u8, i32);
impl_extend_into!(i16, i32);
impl_extend_into!(u16, i32);
impl_extend_into!(i8, i64);
impl_extend_into!(u8, i64);
impl_extend_into!(i16, i64);
impl_extend_into!(u16, i64);
impl_extend_into!(i32, i64);
impl_extend_into!(u32, i64);
impl_extend_into!(u32, u64);
impl_extend_into!(i32, f32);
impl_extend_into!(i32, f64);
impl_extend_into!(u32, f32);
impl_extend_into!(u32, f64);
impl_extend_into!(i64, f64);
impl_extend_into!(u64, f64);
impl_extend_into!(f32, f64);

impl_extend_into!(i32, f32, F32);
impl_extend_into!(i32, f64, F64);
impl_extend_into!(u32, f32, F32);
impl_extend_into!(u32, f64, F64);
impl_extend_into!(i64, f64, F64);
impl_extend_into!(u64, f64, F64);
impl_extend_into!(f32, f64, F64);

impl ExtendInto<F64> for F32 {
	fn extend_into(self) -> F64 {
		(f32::from(self) as f64).into()
	}
}

macro_rules! impl_transmute_into_self {
	($type: ident) => {
		impl TransmuteInto<$type> for $type {
			fn transmute_into(self) -> $type {
				self
			}
		}
	}
}

impl_transmute_into_self!(i32);
impl_transmute_into_self!(i64);
impl_transmute_into_self!(f32);
impl_transmute_into_self!(f64);
impl_transmute_into_self!(F32);
impl_transmute_into_self!(F64);

macro_rules! impl_transmute_into_as {
	($from: ident, $into: ident) => {
		impl TransmuteInto<$into> for $from {
			fn transmute_into(self) -> $into {
				self as $into
			}
		}
	}
}

impl_transmute_into_as!(i8, u8);
impl_transmute_into_as!(i32, u32);
impl_transmute_into_as!(i64, u64);

macro_rules! impl_transmute_into_npf {
	($npf:ident, $float:ident, $signed:ident, $unsigned:ident) => {
		impl TransmuteInto<$float> for $npf {
			fn transmute_into(self) -> $float {
				self.into()
			}
		}

		impl TransmuteInto<$npf> for $float {
			fn transmute_into(self) -> $npf {
				self.into()
			}
		}

		impl TransmuteInto<$signed> for $npf {
			fn transmute_into(self) -> $signed {
				self.to_bits() as _
			}
		}

		impl TransmuteInto<$unsigned> for $npf {
			fn transmute_into(self) -> $unsigned {
				self.to_bits()
			}
		}

		impl TransmuteInto<$npf> for $signed {
			fn transmute_into(self) -> $npf {
				$npf::from_bits(self as _)
			}
		}

		impl TransmuteInto<$npf> for $unsigned {
			fn transmute_into(self) -> $npf {
				$npf::from_bits(self)
			}
		}
	};
}

impl_transmute_into_npf!(F32, f32, i32, u32);
impl_transmute_into_npf!(F64, f64, i64, u64);

impl TransmuteInto<i32> for f32 {
	fn transmute_into(self) -> i32 { self.to_bits() as i32 }
}

impl TransmuteInto<i64> for f64 {
	fn transmute_into(self) -> i64 { self.to_bits() as i64 }
}

impl TransmuteInto<f32> for i32 {
	fn transmute_into(self) -> f32 { f32::from_bits(self as u32) }
}

impl TransmuteInto<f64> for i64 {
	fn transmute_into(self) -> f64 { f64::from_bits(self as u64) }
}

impl TransmuteInto<i32> for u32 {
	fn transmute_into(self) -> i32 { unsafe { transmute(self) } }
}

impl TransmuteInto<i64> for u64 {
	fn transmute_into(self) -> i64 { unsafe { transmute(self) } }
}

impl LittleEndianConvert for i8 {
	fn into_little_endian(self) -> Vec<u8> {
		vec![self as u8]
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		buffer.get(0)
			.map(|v| *v as i8)
			.ok_or_else(|| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for u8 {
	fn into_little_endian(self) -> Vec<u8> {
		vec![self]
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		buffer.get(0)
			.cloned()
			.ok_or_else(|| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for i16 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(2);
		vec.write_i16::<LittleEndian>(self)
			.expect("i16 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_i16::<LittleEndian>()
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for u16 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(2);
		vec.write_u16::<LittleEndian>(self)
			.expect("u16 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_u16::<LittleEndian>()
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for i32 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(4);
		vec.write_i32::<LittleEndian>(self)
			.expect("i32 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_i32::<LittleEndian>()
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for u32 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(4);
		vec.write_u32::<LittleEndian>(self)
			.expect("u32 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_u32::<LittleEndian>()
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for i64 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(8);
		vec.write_i64::<LittleEndian>(self)
			.expect("i64 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_i64::<LittleEndian>()
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for f32 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(4);
		vec.write_f32::<LittleEndian>(self)
			.expect("f32 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_u32::<LittleEndian>()
			.map(f32::from_bits)
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for f64 {
	fn into_little_endian(self) -> Vec<u8> {
		let mut vec = Vec::with_capacity(8);
		vec.write_f64::<LittleEndian>(self)
			.expect("i64 is written without any errors");
		vec
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		io::Cursor::new(buffer).read_u64::<LittleEndian>()
			.map(f64::from_bits)
			.map_err(|_| Error::InvalidLittleEndianBuffer)
	}
}

impl LittleEndianConvert for F32 {
	fn into_little_endian(self) -> Vec<u8> {
		(self.to_bits() as i32).into_little_endian()
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		i32::from_little_endian(buffer).map(|val| Self::from_bits(val as _))
	}
}

impl LittleEndianConvert for F64 {
	fn into_little_endian(self) -> Vec<u8> {
		(self.to_bits() as i64).into_little_endian()
	}

	fn from_little_endian(buffer: &[u8]) -> Result<Self, Error> {
		i64::from_little_endian(buffer).map(|val| Self::from_bits(val as _))
	}
}

macro_rules! impl_integer_arithmetic_ops {
	($type: ident) => {
		impl ArithmeticOps<$type> for $type {
			fn add(self, other: $type) -> $type { self.wrapping_add(other) }
			fn sub(self, other: $type) -> $type { self.wrapping_sub(other) }
			fn mul(self, other: $type) -> $type { self.wrapping_mul(other) }
			fn div(self, other: $type) -> Result<$type, TrapKind> {
				if other == 0 {
					Err(TrapKind::DivisionByZero)
				}
				else {
					let (result, overflow) = self.overflowing_div(other);
					if overflow {
						Err(TrapKind::InvalidConversionToInt)
					} else {
						Ok(result)
					}
				}
			}
		}
	}
}

impl_integer_arithmetic_ops!(i32);
impl_integer_arithmetic_ops!(u32);
impl_integer_arithmetic_ops!(i64);
impl_integer_arithmetic_ops!(u64);

macro_rules! impl_float_arithmetic_ops {
	($type: ident) => {
		impl ArithmeticOps<$type> for $type {
			fn add(self, other: $type) -> $type { self + other }
			fn sub(self, other: $type) -> $type { self - other }
			fn mul(self, other: $type) -> $type { self * other }
			fn div(self, other: $type) -> Result<$type, TrapKind> { Ok(self / other) }
		}
	}
}

impl_float_arithmetic_ops!(f32);
impl_float_arithmetic_ops!(f64);
impl_float_arithmetic_ops!(F32);
impl_float_arithmetic_ops!(F64);

macro_rules! impl_integer {
	($type: ident) => {
		impl Integer<$type> for $type {
			fn leading_zeros(self) -> $type { self.leading_zeros() as $type }
			fn trailing_zeros(self) -> $type { self.trailing_zeros() as $type }
			fn count_ones(self) -> $type { self.count_ones() as $type }
			fn rotl(self, other: $type) -> $type { self.rotate_left(other as u32) }
			fn rotr(self, other: $type) -> $type { self.rotate_right(other as u32) }
			fn rem(self, other: $type) -> Result<$type, TrapKind> {
				if other == 0 { Err(TrapKind::DivisionByZero) }
				else { Ok(self.wrapping_rem(other)) }
			}
		}
	}
}

impl_integer!(i32);
impl_integer!(u32);
impl_integer!(i64);
impl_integer!(u64);

macro_rules! impl_float {
	($type:ident, $int_type:ident) => {
		impl_float!($type, $type, $int_type);
	};
	($type:ident, $intermediate:ident, $int_type:ident) => {
		impl Float<$type> for $type {
			fn abs(self) -> $type {
				$intermediate::abs(self.into()).into()
			}
			fn floor(self) -> $type {
				$intermediate::floor(self.into()).into()
			}
			fn ceil(self) -> $type {
				$intermediate::ceil(self.into()).into()
			}
			fn trunc(self) -> $type {
				$intermediate::trunc(self.into()).into()
			}
			fn round(self) -> $type {
				$intermediate::round(self.into()).into()
			}
			fn nearest(self) -> $type {
				let round = self.round();
				if self.fract().abs() != 0.5 {
					return round;
				}

				use std::ops::Rem;
				if round.rem(2.0) == 1.0 {
					self.floor()
				} else if round.rem(2.0) == -1.0 {
					self.ceil()
				} else {
					round
				}
			}
			fn sqrt(self) -> $type {
				$intermediate::sqrt(self.into()).into()
			}
			// This instruction corresponds to what is sometimes called "minNaN" in other languages.
			fn min(self, other: $type) -> $type {
				if self.is_nan() || other.is_nan() {
					return ::std::$intermediate::NAN.into();
				}

				self.min(other)
			}
			// This instruction corresponds to what is sometimes called "maxNaN" in other languages.
			fn max(self, other: $type) -> $type {
				if self.is_nan() || other.is_nan() {
					return ::std::$intermediate::NAN.into();
				}

				self.max(other)
			}
			fn copysign(self, other: $type) -> $type {
				use std::mem::size_of;

				if self.is_nan() {
					return self;
				}

				let sign_mask: $int_type = 1 << ((size_of::<$int_type>() << 3) - 1);
				let self_int: $int_type = self.transmute_into();
				let other_int: $int_type = other.transmute_into();
				let is_self_sign_set = (self_int & sign_mask) != 0;
				let is_other_sign_set = (other_int & sign_mask) != 0;
				if is_self_sign_set == is_other_sign_set {
					self
				} else if is_other_sign_set {
					(self_int | sign_mask).transmute_into()
				} else {
					(self_int & !sign_mask).transmute_into()
				}
			}
		}
	};
}

impl_float!(f32, i32);
impl_float!(f64, i64);
impl_float!(F32, f32, i32);
impl_float!(F64, f64, i64);
