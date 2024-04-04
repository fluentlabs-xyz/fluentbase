use crate::{BufferDecoder, Encoder, WritableBuffer};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EmptyVec;

impl Encoder<EmptyVec> for EmptyVec {
    const HEADER_SIZE: usize = 12;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        // first 4 bytes are number of elements
        encoder.write_u32(field_offset, 0);
        // remaining 4+4 are offset and length
        encoder.write_bytes(field_offset + 4, &[]);
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        _result: &mut EmptyVec,
    ) -> (usize, usize) {
        let count = decoder.read_u32(field_offset);
        debug_assert_eq!(count, 0);
        decoder.read_bytes_header(field_offset + 4)
    }
}

#[cfg(test)]
mod tests {
    use crate::{define_codec_struct, BufferDecoder, EmptyVec, Encoder};
    use alloy_primitives::Bytes;

    define_codec_struct! {
        pub struct ContractOutput {
            return_data: Bytes,
            logs: Vec<ContractOutput>,
        }
    }
    define_codec_struct! {
        pub struct ContractOutputNoLogs {
            return_data: Bytes,
            logs: EmptyVec,
        }
    }

    #[test]
    fn test_empty_encode() {
        let input = ContractOutputNoLogs {
            return_data: Bytes::copy_from_slice("Hello, World".as_bytes()),
            logs: Default::default(),
        };
        let buffer = input.encode_to_vec(0);
        let mut buffer_decoder = BufferDecoder::new(&buffer);
        let mut output = ContractOutput::default();
        ContractOutput::decode_body(&mut buffer_decoder, 0, &mut output);
        assert_eq!(input.return_data, output.return_data);
        assert_eq!(output.logs, vec![]);
    }
}
