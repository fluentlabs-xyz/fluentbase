use crate::{BufferDecoder, BufferEncoder, Encoder};
use alloy_primitives::{Address, FixedBytes, Uint};

impl<const N: usize> Encoder<FixedBytes<N>> for FixedBytes<N> {
    const HEADER_SIZE: usize = N;
    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        self.0.encode(encoder, field_offset)
    }
    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut FixedBytes<N>,
    ) -> (usize, usize) {
        <[u8; N]>::decode_body(decoder, field_offset, &mut result.0);
        (0, 0)
    }
}

macro_rules! impl_evm_fixed {
    ($typ:ty) => {
        impl Encoder<$typ> for $typ {
            const HEADER_SIZE: usize = <$typ>::len_bytes();
            fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
                self.0.encode(encoder, field_offset)
            }
            fn decode_header(
                decoder: &mut BufferDecoder,
                field_offset: usize,
                result: &mut $typ,
            ) -> (usize, usize) {
                FixedBytes::<{ Self::HEADER_SIZE }>::decode_header(
                    decoder,
                    field_offset,
                    &mut result.0,
                );
                (0, 0)
            }
        }
    };
}

impl_evm_fixed!(Address);

impl<const BITS: usize, const LIMBS: usize> Encoder<Uint<BITS, LIMBS>> for Uint<BITS, LIMBS> {
    const HEADER_SIZE: usize = Self::BYTES;
    fn encode(&self, encoder: &mut BufferEncoder, field_offset: usize) {
        self.as_limbs().encode(encoder, field_offset)
    }
    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut Uint<BITS, LIMBS>,
    ) -> (usize, usize) {
        unsafe {
            <[u64; LIMBS]>::decode_header(decoder, field_offset, result.as_limbs_mut());
        }
        (0, 0)
    }
}
