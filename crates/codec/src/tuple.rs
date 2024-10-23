use crate::{
    alloc::string::ToString,
    encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
    error::{CodecError, DecodingError},
};
use byteorder::ByteOrder;
use bytes::{Buf, BytesMut};
impl<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool> Encoder<B, { ALIGN }, { SOL_MODE }>
    for ()
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

impl<T, B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool> Encoder<B, { ALIGN }, { SOL_MODE }>
    for (T,)
where
    T: Encoder<B, { ALIGN }, { SOL_MODE }>,
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

const WORD_SIZE: usize = 32;
const U32_SIZE: usize = 4;

const fn is_power_of_two(n: usize) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

macro_rules! impl_encoder_for_tuple {
    ($($T:ident),+; $($idx:tt),+; $is_solidity:expr) => {
        impl<B: ByteOrder, const ALIGN: usize, $($T,)+> Encoder<B, {ALIGN}, $is_solidity> for ($($T,)+)
        where
            $($T: Encoder<B, {ALIGN}, $is_solidity>,)+
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

                let mut current_offset = offset;

                if Self::IS_DYNAMIC {
                    let buf_len = buf.len();

                    let dynamic_offset = if buf_len == 0 {
                        if $is_solidity { WORD_SIZE } else { U32_SIZE }
                    } else {
                        buf_len
                    };
                    write_u32_aligned::<B, ALIGN>(buf, current_offset, dynamic_offset as u32);
                    current_offset += if $is_solidity { WORD_SIZE } else { U32_SIZE };


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
                    current_offset = 0;
                    $(

                        self.$idx.encode(&mut tmp, current_offset)?;
                        current_offset += if $T::IS_DYNAMIC && $is_solidity {
                           WORD_SIZE
                        } else {
                            align_up::<ALIGN>($T::HEADER_SIZE)
                        };
                    )+
                    buf.unsplit(tmp);
                } else {
                    $(
                        self.$idx.encode(buf, current_offset)?;
                        current_offset += align_up::<ALIGN>($T::HEADER_SIZE);
                    )+
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

                let mut current_offset = 0;

                Ok(($(
                    {
                        let value = $T::decode(&tmp, current_offset)?;
                        current_offset += if $T::IS_DYNAMIC && $is_solidity{
                            WORD_SIZE
                        } else {
                            align_up::<ALIGN>($T::HEADER_SIZE)
                        };
                        value
                    },
                )+))
            }

            fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::LittleEndian;
    use bytes::BytesMut;

    #[test]
    fn test_empty_tuple() {
        let t = ();
        let mut buf = BytesMut::new();

        <() as Encoder<LittleEndian, 4, false>>::encode(&t, &mut buf, 0).unwrap();
        let encoded = buf.freeze();
        assert_eq!(hex::encode(&encoded), "");
        let decoded = <() as Encoder<LittleEndian, 4, false>>::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, ());
    }

    #[test]
    fn test_single_element_tuple() {
        let original: (u32,) = (100u32,);
        let mut buf = BytesMut::new();
        <(u32,) as Encoder<LittleEndian, 4, false>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        assert_eq!(hex::encode(&encoded), "64000000");

        let decoded = <(u32,) as Encoder<LittleEndian, 4, false>>::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_simple_tuple() {
        type Tuple = (u32, u16);
        let original: Tuple = (100u32, 20u16);
        let mut buf = BytesMut::new();
        <Tuple as Encoder<LittleEndian, 4, false>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("{:?}", encoded);
        assert_eq!(hex::encode(&encoded), "6400000014000000");

        let decoded = <Tuple as Encoder<LittleEndian, 4, false>>::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_big_tuple() {
        type Tuple = (u32, u16, u8, u64, u32, u16, u8, u64);
        let original: Tuple = (100u32, 20u16, 30u8, 40u64, 50u32, 60u16, 70u8, 80u64);
        let mut buf = BytesMut::new();
        <Tuple as Encoder<LittleEndian, 4, false>>::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        println!("{:?}", hex::encode(&encoded));
        assert_eq!(
            hex::encode(&encoded),
            "64000000140000001e0000002800000000000000320000003c000000460000005000000000000000"
        );

        let decoded = <Tuple as Encoder<LittleEndian, 4, false>>::decode(&encoded, 0).unwrap();
        assert_eq!(decoded, original);
    }
}
