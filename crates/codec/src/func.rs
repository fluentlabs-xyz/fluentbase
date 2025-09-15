use crate::encoder::Encoder;
use crate::error::CodecError;
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};

/// Trait for types that can be used as function arguments
/// Only implemented for tuples
pub trait FunctionArgs<
    B: ByteOrder,
    const ALIGN: usize,
    const SOL_MODE: bool,
    const IS_STATIC: bool,
>: Encoder<B, ALIGN, SOL_MODE, IS_STATIC>
{
    /// Encode without the outer tuple offset for dynamic types
    fn encode_as_args(&self, buf: &mut BytesMut) -> Result<(), CodecError> {
        if Self::IS_DYNAMIC {
            // For dynamic types, skip the first ALIGN bytes (the tuple offset)
            let mut temp_buf = BytesMut::new();
            self.encode(&mut temp_buf, 0)?;

            // Skip the offset and append the rest
            if temp_buf.len() > ALIGN {
                buf.extend_from_slice(&temp_buf[ALIGN..]);
            }
            Ok(())
        } else {
            // Static types encode normally
            self.encode(buf, buf.len())
        }
    }

    /// Decode expecting no outer tuple offset for dynamic types
    fn decode_as_args(buf: &impl Buf) -> Result<Self, CodecError> {
        if Self::IS_DYNAMIC {
            use bytes::BufMut;
            // Add the offset back for proper decoding
            let mut temp_buf = BytesMut::with_capacity(ALIGN + buf.remaining());

            // Add offset header
            if SOL_MODE {
                // Solidity: 32-byte aligned, big-endian
                temp_buf.resize(ALIGN, 0);
                let offset = (ALIGN as u32).to_be_bytes();
                temp_buf[ALIGN - 4..ALIGN].copy_from_slice(&offset);
            } else {
                // Fluent: 4-byte, little-endian
                temp_buf.put_u32_le(ALIGN as u32);
            }

            // Add the actual data
            temp_buf.extend_from_slice(buf.chunk());

            // Decode with the reconstructed offset
            Self::decode(&temp_buf.freeze(), 0)
        } else {
            // Static types decode normally
            Self::decode(buf, 0)
        }
    }
}

// Implement for all tuples - super simple!
impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    FunctionArgs<B, ALIGN, SOL_MODE, IS_STATIC> for ()
{
}

impl<T, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    FunctionArgs<B, ALIGN, SOL_MODE, IS_STATIC> for (T,)
where
    T: Encoder<B, ALIGN, SOL_MODE, IS_STATIC>,
{
}

// Macro for Solidity mode (SOL_MODE = true)
macro_rules! impl_function_args_solidity {
    ($($T:ident),+) => {
        impl<$($T,)+ B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool>
            FunctionArgs<B, ALIGN, true, IS_STATIC> for ($($T,)+)
        where
            $($T: Encoder<B, ALIGN, true, IS_STATIC>,)+
        {}
    };
}

// Macro for Compact mode (SOL_MODE = false)
macro_rules! impl_function_args_compact {
    ($($T:ident),+) => {
        impl<$($T,)+ B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool>
            FunctionArgs<B, ALIGN, false, IS_STATIC> for ($($T,)+)
        where
            $($T: Encoder<B, ALIGN, false, IS_STATIC>,)+
        {}
    };
}

impl_function_args_solidity!(T0, T1);
impl_function_args_compact!(T0, T1);

impl_function_args_solidity!(T0, T1, T2);
impl_function_args_compact!(T0, T1, T2);

impl_function_args_solidity!(T0, T1, T2, T3);
impl_function_args_compact!(T0, T1, T2, T3);

impl_function_args_solidity!(T0, T1, T2, T3, T4);
impl_function_args_compact!(T0, T1, T2, T3, T4);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5, T6);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5, T6);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5, T6, T7);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5, T6, T7);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5, T6, T7, T8);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);

impl_function_args_solidity!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_function_args_compact!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
