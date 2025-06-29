use crate::{
    align_up,
    optimized::{
        ctx::EncodingContext,
        encoder::Encoder,
        error::CodecError,
        utils::{read_u32_aligned, write_u32_aligned},
    },
};
use byteorder::ByteOrder;
use bytes::{Buf, BufMut};

// Empty tuple implementation
impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool> Encoder<B, ALIGN, SOL_MODE> for () {
    const HEADER_SIZE: usize = 0;
    const IS_DYNAMIC: bool = false;

    fn header_size(&self) -> usize {
        0
    }

    fn encode_header(
        &self,
        _buf: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        Ok(0)
    }

    fn encode_tail(
        &self,
        _buf: &mut impl BufMut,
        _ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        Ok(0)
    }

    fn decode(_buf: &impl Buf, _offset: usize) -> Result<Self, CodecError> {
        Ok(())
    }
}

// Single element tuple
impl<T, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool> Encoder<B, ALIGN, SOL_MODE> for (T,)
where
    T: Encoder<B, ALIGN, SOL_MODE>,
{
    const HEADER_SIZE: usize = T::HEADER_SIZE;
    const IS_DYNAMIC: bool = T::IS_DYNAMIC;

    fn header_size(&self) -> usize {
        let size = self.0.header_size();
        if Self::IS_DYNAMIC {
            size + if SOL_MODE { 32 } else { align_up::<ALIGN>(4) }
        } else {
            size
        }
    }

    fn encode_header(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut total_written = 0;

        // If this tuple is dynamic, write offset first
        if Self::IS_DYNAMIC {
            let offset = if SOL_MODE { 32 } else { align_up::<ALIGN>(4) };
            write_u32_aligned::<B, ALIGN>(buf, offset as u32);
            total_written += offset;

            // Adjust context for inner element
            ctx.hdr_ptr += offset as u32;
        }

        total_written += self.0.encode_header(buf, ctx)?;
        Ok(total_written)
    }

    fn encode_tail(
        &self,
        buf: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        self.0.encode_tail(buf, ctx)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let mut current_offset = offset;

        // If tuple is dynamic, read the offset first
        if Self::IS_DYNAMIC {
            let data_offset = read_u32_aligned::<B, ALIGN>(buf, current_offset)? as usize;
            // For dynamic tuples, the actual data starts at offset + data_offset
            current_offset = offset + data_offset;
        }

        Ok((T::decode(buf, current_offset)?,))
    }
}

// Macro for multiple element tuples
macro_rules! impl_encoder_for_tuple {
    ($($T:ident),+; $($idx:tt),+) => {
        impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, $($T,)+>
            Encoder<B, ALIGN, SOL_MODE> for ($($T,)+)
        where
            $($T: Encoder<B, ALIGN, SOL_MODE>,)+
        {


            const HEADER_SIZE: usize = {
                let mut size = 0;
                $(
                    size += $T::HEADER_SIZE;
                )+
                // If tuple is dynamic, add space for offset
                if Self::IS_DYNAMIC {
                    size += if SOL_MODE { 32 } else { align_up::<ALIGN>(4) };
                }
                size
            };

            const IS_DYNAMIC: bool = {
                let mut is_dynamic = false;
                $(
                    is_dynamic |= $T::IS_DYNAMIC;
                )+
                is_dynamic
            };

            /// Calculate header size for all tuple elements
            fn header_size(&self) -> usize {
                let mut size = 0;
                $(
                    size += self.$idx.header_size();
                )+
                // If tuple is dynamic, add space for offset
                if Self::IS_DYNAMIC {
                    size += if SOL_MODE { 32 } else { align_up::<ALIGN>(4) };
                }
                size
            }

            /// Encode headers for all tuple elements
            fn encode_header(
                &self,
                buf: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                let mut total_written = 0;

                // If this tuple is dynamic, write offset first
                if Self::IS_DYNAMIC {
                    let offset = if SOL_MODE { 32 } else { align_up::<ALIGN>(4) };
                    write_u32_aligned::<B, ALIGN>(buf, offset as u32);
                    total_written += if SOL_MODE { 32 } else { align_up::<ALIGN>(4) };

                    // Adjust context for inner elements
                    ctx.hdr_ptr += offset as u32;
                }

                $(
                    total_written += self.$idx.encode_header(buf, ctx)?;
                )+

                Ok(total_written)
            }

            /// Encode tails for dynamic elements only
            fn encode_tail(
                &self,
                buf: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                let mut total_written = 0;

                $(
                    if $T::IS_DYNAMIC {
                        total_written += self.$idx.encode_tail(buf, ctx)?;
                    }
                )+

                Ok(total_written)
            }

            /// Decode tuple from buffer
            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                // Determine the actual start position of tuple data
                let tuple_start = if Self::IS_DYNAMIC {
                    // Read offset and calculate absolute position
                    let data_offset = read_u32_aligned::<B, ALIGN>(buf, offset)? as usize;
                    offset + data_offset
                } else {
                    offset
                };
            
                // Now decode elements sequentially from tuple_start
                let mut current_offset = tuple_start;
            
                Ok(($(
                    {
                        let value = $T::decode(buf, current_offset)?;
                        
                        // Move to next element using the actual header size
                        current_offset += value.header_size();
                        
                        value
                    },
                )+))
            }

            /// Calculate total size of tail data
            fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
                let mut size = 0;
                $(
                    if $T::IS_DYNAMIC {
                        size += self.$idx.tail_size(ctx)?;
                    }
                )+
                Ok(size)
            }
        }
    };
}

// Generate implementations for tuples of different sizes
impl_encoder_for_tuple!(T1, T2; 0, 1);
impl_encoder_for_tuple!(T1, T2, T3; 0, 1, 2);
impl_encoder_for_tuple!(T1, T2, T3, T4; 0, 1, 2, 3);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5; 0, 1, 2, 3, 4);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6; 0, 1, 2, 3, 4, 5);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7; 0, 1, 2, 3, 4, 5, 6);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8; 0, 1, 2, 3, 4, 5, 6, 7);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9; 0, 1, 2, 3, 4, 5, 6, 7, 8);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimized::utils::test_utils::{assert_codec_compact, assert_codec_sol};
    use alloy_primitives::{address, U256};

    mod compact {
        use super::*;
        use alloy_primitives::Address;

        #[test]
        fn test_empty_tuple() {
            let value = ();
            assert_codec_compact("", "", &value);
        }

        #[test]
        fn test_single_element_tuple() {
            let value = (100u32,);
            assert_codec_compact("64000000", "", &value);
        }

        #[test]
        fn test_simple_tuple() {
            let value = (100u32, 20u16);
            assert_codec_compact(
                concat!(
                    "64000000", // 100 (u32)
                    "14000000", // 20 (u16, padded to 4 bytes)
                ),
                "",
                &value,
            );
        }

        #[test]
        fn test_big_tuple() {
            let value = (100u32, 20u16, 30u8, 40u64, 50u32, 60u16, 70u8, 80u64);
            assert_codec_compact(
                concat!(
                    "64000000",         // 100 (u32)
                    "14000000",         // 20 (u16)
                    "1e000000",         // 30 (u8)
                    "2800000000000000", // 40 (u64)
                    "32000000",         // 50 (u32)
                    "3c000000",         // 60 (u16)
                    "46000000",         // 70 (u8)
                    "5000000000000000", // 80 (u64)
                ),
                "",
                &value,
            );
        }

        // print_encoded::<LittleEndian, 4>(hex::decode("").unwrap());
        #[test]
        fn test_complex_tuple_fluent() {
            let msg = "Hello World".to_string();
            let contract_address = address!("f91c20c0cafbfdc150adff51bbfc5808edde7cb5");
            let value = U256::from(1);
            let gas_limit = 21_000u64;

            type TestTuple = (String, Address, U256, u64);
            let original: TestTuple = (msg, contract_address, value, gas_limit);

            assert_codec_compact(
                concat!(
                    "04000000", // [0x0000] 0 = 4
                    "44000000", // [0x0004] 4 = 68
                    "0b000000", // [0x0008] 8 = 11
                    "f91c20c0", // [0x000c] 12 = 3223330041
                    "cafbfdc1", // [0x0010] 16 = 3254647754
                    "50adff51", // [0x0014] 20 = 1375710544
                    "bbfc5808", // [0x0018] 24 = 140049595
                    "edde7cb5", // [0x001c] 28 = 3044859629
                    "01000000", // [0x0020] 32 = 1
                    "00000000", // [0x0024] 36 = 0
                    "00000000", // [0x0028] 40 = 0
                    "00000000", // [0x002c] 44 = 0
                    "00000000", // [0x0030] 48 = 0
                    "00000000", // [0x0034] 52 = 0
                    "00000000", // [0x0038] 56 = 0
                    "00000000", // [0x003c] 60 = 0
                    "08520000", // [0x0040] 64 = 21000
                    "00000000", // [0x0044] 68 = 0
                ),
                concat!(
                    // string bytes (11 bytes + padding to 4-byte boundary)
                    "48656c6c", // [0x0048] 72 = 1819043144
                    "6f20576f", // [0x004c] 76 = 1867980911
                    "726c6400", // [0x0050] 80 = 6581362
                ),
                &original,
            );
        }
    }

    mod solidity {
        use super::*;
        use crate::optimized::utils::test_utils::{encode_alloy_sol, print_encoded};
        use byteorder::BigEndian;

        #[test]
        fn test_empty_tuple() {
            let value = ();
            assert_codec_sol("", "", &value);
        }

        #[test]
        fn test_single_element_tuple() {
            let value = (100u32,);
            assert_codec_sol(
                "0000000000000000000000000000000000000000000000000000000000000064",
                "",
                &value,
            );
        }

        #[test]
        fn test_simple_tuple() {
            let value = (100u32, 20u32);
            assert_codec_sol(
                concat!(
                    "0000000000000000000000000000000000000000000000000000000000000064", // 100
                    "0000000000000000000000000000000000000000000000000000000000000014", // 20
                ),
                "",
                &value,
            );
        }

        #[test]
        fn test_complex_tuple_with_dynamic() {
            let msg = "Hello World".to_string();
            let contract_address = address!("f91c20c0cafbfdc150adff51bbfc5808edde7cb5");
            let value = U256::from(1);
            let gas_limit = 21_000u64;

            let original = (msg, contract_address, value, gas_limit);

            let expected_encoded = encode_alloy_sol(&original);
            print_encoded::<BigEndian, 32>(&expected_encoded);

            assert_codec_sol(
                concat!(
                    "0000000000000000000000000000000000000000000000000000000000000020", /* [0x0000] 0 = 32 */
                    "0000000000000000000000000000000000000000000000000000000000000080", /* [0x0020] 32 = 128 */
                    "000000000000000000000000f91c20c0cafbfdc150adff51bbfc5808edde7cb5", /* [0x0040] 64 = 3990781109 */
                    "0000000000000000000000000000000000000000000000000000000000000001", /* [0x0060] 96 = 1 */
                    "0000000000000000000000000000000000000000000000000000000000005208", /* [0x0080] 128 = 21000 */
                ),
                concat!(
                    "000000000000000000000000000000000000000000000000000000000000000b", /* [0x00a0] 160 = 11 */
                    "48656c6c6f20576f726c64000000000000000000000000000000000000000000", /* [0x00c0] 192 = 0 */
                ),
                &original,
            );
        }
    }
}
