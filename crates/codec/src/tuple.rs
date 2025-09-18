use crate::{
    alloc::string::ToString,
    encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};

impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for ()
{
    const HEADER_SIZE: usize = 0;
    const IS_DYNAMIC: bool = false;

    fn encode(&self, _buf: &mut BytesMut, _offset: usize) -> Result<(), CodecError> {
        Ok(())
    }

    fn decode(_buf: &impl Buf, _offset: usize) -> Result<Self, CodecError> {
        Ok(())
    }

    fn partial_decode(_buf: &impl Buf, _offset: usize) -> Result<(usize, usize), CodecError> {
        Ok((0, 0))
    }
}

impl<T, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>
    Encoder<B, ALIGN, SOL_MODE, IS_STATIC> for (T,)
where
    T: Encoder<B, ALIGN, SOL_MODE, IS_STATIC>,
{
    const HEADER_SIZE: usize = align_up::<ALIGN>(T::HEADER_SIZE);
    const IS_DYNAMIC: bool = T::IS_DYNAMIC;

    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
        let mut current_offset = offset;
        let header_el_size = if SOL_MODE {
            align_up::<ALIGN>(32)
        } else {
            align_up::<ALIGN>(4)
        };
        if Self::IS_DYNAMIC {
            let buf_len = buf.len();
            let dynamic_offset = if buf_len == 0 {
                header_el_size
            } else {
                buf_len
            };
            write_u32_aligned::<B, ALIGN>(buf, current_offset, dynamic_offset as u32);
            current_offset += header_el_size;

            let aligned_header_size = align_up::<ALIGN>(T::HEADER_SIZE);
            if buf_len < current_offset + aligned_header_size {
                buf.resize(current_offset + aligned_header_size, 0);
            }
            let mut tmp = buf.split_off(current_offset);

            self.0.encode(&mut tmp, 0)?;
            buf.unsplit(tmp);
        } else {
            self.0.encode(buf, current_offset)?;
        }

        Ok(())
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
        let chunk = if Self::IS_DYNAMIC {
            let dynamic_offset = read_u32_aligned::<B, ALIGN>(&buf.chunk(), offset)? as usize;
            &buf.chunk()[dynamic_offset..]
        } else {
            &buf.chunk()[offset..]
        };

        Ok((T::decode(&chunk, 0)?,))
    }

    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
        T::partial_decode(buf, offset)
    }
}
const WORD_SIZE_SOL: usize = 32;
const WORD_SIZE_DEFAULT: usize = 4;

const fn is_power_of_two(n: usize) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

macro_rules! impl_encoder_for_tuple {
    ($($T:ident),+; $($idx:tt),+; $is_solidity:expr) => {
        #[allow(unused_assignments)]
        impl<B: ByteOrder, const ALIGN: usize, const IS_STATIC: bool, $($T,)+>
        Encoder<B, ALIGN, $is_solidity, IS_STATIC> for ($($T,)+)
        where
            $($T: Encoder<B, ALIGN, $is_solidity, IS_STATIC>,)+
        {
            const HEADER_SIZE: usize = {
                let mut size = 0;
                $(
                    size = align_up::<ALIGN>(size);
                    size += $T::HEADER_SIZE;
                )+
                align_up::<ALIGN>(size)
            };

            const IS_DYNAMIC: bool = {
                let mut is_dynamic = false;
                $(
                    is_dynamic |= $T::IS_DYNAMIC;
                )+
                is_dynamic
            };


            fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
                assert!(is_power_of_two(ALIGN), "ALIGN must be a power of two");


                if $is_solidity {
                    // Solidity mode
                    let aligned_offset = align_up::<ALIGN>(offset);
                    let is_dynamic = Self::IS_DYNAMIC;

                    let aligned_header_size = {
                        let mut size = 0;
                        $(
                            size += if $T::IS_DYNAMIC {
                                align_up::<ALIGN>(4)
                            } else {
                                align_up::<ALIGN>($T::HEADER_SIZE)
                            };
                        )+
                        size
                    };

                    let mut tail = if is_dynamic {
                        let buf_len = buf.len();
                        let offset = if buf_len == 0 { align_up::<ALIGN>(4) } else { buf_len };
                        write_u32_aligned::<B, ALIGN>(buf, aligned_offset, offset as u32);
                        if buf.len() < aligned_header_size + offset {
                            buf.resize(aligned_header_size + offset, 0);
                        }
                        buf.split_off(offset)
                    } else {
                        if buf.len() < aligned_offset + aligned_header_size {
                            buf.resize(aligned_offset + aligned_header_size, 0);
                        }
                        buf.split_off(aligned_offset)
                    };

                    let mut tail_offset = 0;
                    $(
                        if $T::IS_DYNAMIC {
                            self.$idx.encode(&mut tail, tail_offset)?;
                            tail_offset += align_up::<ALIGN>(4);
                        } else {
                            self.$idx.encode(&mut tail, tail_offset)?;
                            tail_offset += align_up::<ALIGN>($T::HEADER_SIZE);
                        }
                    )+

                    buf.unsplit(tail);
                } else {
                    // WASM mode
                    let mut current_offset = offset;
                    let header_el_size = align_up::<ALIGN>(4);

                    if Self::IS_DYNAMIC {
                        let buf_len = buf.len();
                        let dynamic_offset = if buf_len == 0 {
                            header_el_size
                        } else {
                            buf_len
                        };
                        write_u32_aligned::<B, ALIGN>(buf, current_offset, dynamic_offset as u32);
                        current_offset += header_el_size;

                        let aligned_header_size = {
                            let mut size = 0;
                            $(
                                size += align_up::<ALIGN>($T::HEADER_SIZE);
                            )+
                            size
                        };

                        if buf_len < current_offset + aligned_header_size {
                            buf.resize(current_offset + aligned_header_size, 0);
                        }
                        let mut tmp = buf.split_off(current_offset);

                        let mut current_tmp_offset = 0;
                        $(
                            self.$idx.encode(&mut tmp, current_tmp_offset)?;
                            current_tmp_offset += align_up::<ALIGN>($T::HEADER_SIZE);
                        )+

                        buf.unsplit(tmp);
                    } else {
                        let mut current_field_offset = current_offset;
                        $(
                            self.$idx.encode(buf, current_field_offset)?;
                            current_field_offset += align_up::<ALIGN>($T::HEADER_SIZE);
                        )+
                    }
                }

                Ok(())
            }

            fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError> {
                if buf.remaining() < offset {
                    return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                        expected: offset,
                        found: buf.remaining(),
                        msg: "buf too small to take offset".to_string(),
                    }));
                }

                let word_size = if $is_solidity { WORD_SIZE_SOL } else { WORD_SIZE_DEFAULT };

                let tmp = if Self::IS_DYNAMIC {
                    let dynamic_offset = read_u32_aligned::<B, ALIGN>(&buf.chunk(), offset)? as usize;
                    if buf.remaining() < dynamic_offset {
                       return Err(CodecError::Decoding(DecodingError::BufferTooSmall {
                            expected: dynamic_offset,
                            found: buf.remaining(),
                            msg: "buf too small to take dynamic offset".to_string(),
                        }));
                    }
                    &buf.chunk()[dynamic_offset..]
                } else {
                    &buf.chunk()[offset..]
                };

                let mut _current_offset = 0;

                Ok(($(
                    {
                        let value = $T::decode(&tmp, _current_offset)?;
                        _current_offset += if $T::IS_DYNAMIC && $is_solidity {
                           word_size
                        } else {
                            align_up::<ALIGN>($T::HEADER_SIZE)
                        };
                        value
                    },
                )+))
            }

            fn partial_decode(_buf: &impl Buf, _offset: usize) -> Result<(usize, usize), CodecError> {
               Ok((0,0))
            }
        }
    };
}

impl_encoder_for_tuple!(T1, T2; 0, 1; true);
impl_encoder_for_tuple!(T1, T2; 0, 1; false);
impl_encoder_for_tuple!(T1, T2, T3; 0, 1, 2; true);
impl_encoder_for_tuple!(T1, T2, T3; 0, 1, 2; false);
impl_encoder_for_tuple!(T1, T2, T3, T4; 0, 1, 2, 3; true);
impl_encoder_for_tuple!(T1, T2, T3, T4; 0, 1, 2, 3; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5; 0, 1, 2, 3, 4; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5; 0, 1, 2, 3, 4; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6; 0, 1, 2, 3, 4, 5; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6; 0, 1, 2, 3, 4, 5; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7; 0, 1, 2, 3, 4, 5, 6; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7; 0, 1, 2, 3, 4, 5, 6; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8; 0, 1, 2, 3, 4, 5, 6, 7; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8; 0, 1, 2, 3, 4, 5, 6, 7; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9; 0, 1, 2, 3, 4, 5, 6, 7, 8; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9; 0, 1, 2, 3, 4, 5, 6, 7, 8; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10; false);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11; true);
impl_encoder_for_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11; false);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompactABI;
    use alloy_primitives::{address, Address, U256};
    use bytes::BytesMut;

    #[test]
    fn test_empty_tuple() {
        let t = ();
        let mut buf = BytesMut::new();

        CompactABI::encode(&t, &mut buf, 0).unwrap();
        let encoded = buf.freeze();
        assert_eq!(hex::encode(&encoded), "");
        let decoded: () = CompactABI::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, ());
    }

    #[test]
    fn test_single_element_tuple() {
        let original: (u32,) = (100u32,);
        let mut buf = BytesMut::new();
        CompactABI::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        assert_eq!(hex::encode(&encoded), "64000000");

        let decoded: (u32,) = CompactABI::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_simple_tuple() {
        type Tuple = (u32, u16);
        let original: Tuple = (100u32, 20u16);
        let mut buf = BytesMut::new();
        CompactABI::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("{:?}", encoded);
        assert_eq!(hex::encode(&encoded), "6400000014000000");

        let decoded: Tuple = CompactABI::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_big_tuple() {
        type Tuple = (u32, u16, u8, u64, u32, u16, u8, u64);
        let original: Tuple = (100u32, 20u16, 30u8, 40u64, 50u32, 60u16, 70u8, 80u64);
        let mut buf = BytesMut::new();
        CompactABI::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("{:?}", hex::encode(&encoded));
        assert_eq!(
            hex::encode(&encoded),
            "64000000140000001e0000002800000000000000320000003c000000460000005000000000000000"
        );

        let decoded: Tuple = CompactABI::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_complex_tuple_fluent() {
        let msg = "Hello World".to_string();
        let contract_address = address!("f91c20c0cafbfdc150adff51bbfc5808edde7cb5");
        let value = U256::from(0);
        let gas_limit = 21_000;

        type TestTuple = (Address, U256, u64, String);
        let original: TestTuple = (contract_address, value, gas_limit, msg);

        let mut buf = BytesMut::new();
        CompactABI::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("Encoded: {}", hex::encode(&encoded));
        let expected_encoded = "04000000f91c20c0cafbfdc150adff51bbfc5808edde7cb500000000000000000000000000000000000000000000000000000000000000000852000000000000440000000b00000048656c6c6f20576f726c6400";

        assert_eq!(hex::encode(&encoded), expected_encoded);
        let decoded: TestTuple = CompactABI::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }
}
