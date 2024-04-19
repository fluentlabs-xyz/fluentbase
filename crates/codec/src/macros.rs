#[cfg(test)]
mod tests {
    use crate::{BufferDecoder, BufferEncoder, Encoder};
    use fluentbase_codec_derive::Codec;
    use hashbrown::HashMap;

    #[derive(Debug, Default, PartialEq, Codec)]
    struct Test {
        a: u16,
        b: u32,
        c: Option<u64>,
    }

    #[test]
    fn test_option_encoding() {
        let test = Test {
            a: 100,
            b: 20,
            c: Some(3),
        };
        let buffer = test.encode_to_vec(0);
        let mut buffer_decoder = BufferDecoder::new(&buffer);
        let mut test2 = Test::default();
        Test::decode_body(&mut buffer_decoder, 0, &mut test2);
        assert_eq!(test, test2);
    }

    #[derive(Default, Debug, Codec, PartialEq)]
    pub struct SimpleType {
        a: u64,
        b: u32,
        c: u16,
    }

    #[test]
    fn test_simple_type() {
        let value0 = SimpleType {
            a: 100,
            b: 20,
            c: 3,
        };
        assert_eq!(SimpleType::HEADER_SIZE, 8 + 4 + 2);
        let encoded_value = {
            let mut buffer_encoder = BufferEncoder::new(SimpleType::HEADER_SIZE, None);
            value0.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&encoded_value));
        let mut buffer_decoder = BufferDecoder::new(encoded_value.as_slice());
        let mut value1 = Default::default();
        SimpleType::decode_body(&mut buffer_decoder, 0, &mut value1);
        assert_eq!(value0, value1);
    }

    #[derive(Default, Debug, Codec, PartialEq)]
    pub struct ComplicatedType {
        values: Vec<SimpleType>,
        maps: HashMap<u32, ComplicatedType>,
    }

    #[test]
    fn test_decode_specific_field() {
        let value = SimpleType {
            a: 100,
            b: 20,
            c: 3,
        };
        // check offsets
        assert_eq!(<SimpleType as ISimpleType>::A::FIELD_OFFSET, 0);
        assert_eq!(<SimpleType as ISimpleType>::B::FIELD_OFFSET, 8);
        assert_eq!(<SimpleType as ISimpleType>::C::FIELD_OFFSET, 8 + 4);
        // check sizes
        assert_eq!(<SimpleType as ISimpleType>::A::FIELD_SIZE, 8);
        assert_eq!(<SimpleType as ISimpleType>::B::FIELD_SIZE, 4);
        assert_eq!(<SimpleType as ISimpleType>::C::FIELD_SIZE, 2);
        // encode entire struct
        let encoded_value = value.encode_to_vec(0);
        let mut encoded_value = encoded_value.as_slice();
        // decode only field `a`
        {
            let mut a: u64 = 0;
            <SimpleType as ISimpleType>::A::decode_field_header(&mut encoded_value, &mut a);
            assert_eq!(a, value.a);
        }
        // decode only field `b`
        {
            let mut b: u32 = 0;
            <SimpleType as ISimpleType>::B::decode_field_header(&mut encoded_value, &mut b);
            assert_eq!(b, value.b);
        }
        // decode only field `c`
        {
            let mut c: u16 = 0;
            <SimpleType as ISimpleType>::C::decode_field_header(&mut encoded_value, &mut c);
            assert_eq!(c, value.c);
        }
    }

    #[test]
    fn test_complicated_type() {
        let value0 = ComplicatedType {
            values: vec![
                SimpleType {
                    a: 100,
                    b: 20,
                    c: 3,
                },
                SimpleType {
                    a: u64::MAX,
                    b: u32::MAX,
                    c: u16::MAX,
                },
            ],
            maps: HashMap::from([(
                7,
                ComplicatedType {
                    values: vec![
                        SimpleType { a: 1, b: 2, c: 3 },
                        SimpleType { a: 4, b: 5, c: 6 },
                    ],
                    maps: Default::default(),
                },
            )]),
        };
        assert_eq!(
            ComplicatedType::HEADER_SIZE,
            Vec::<SimpleType>::HEADER_SIZE + HashMap::<u32, SimpleType>::HEADER_SIZE
        );
        let encoded_value = {
            let mut buffer_encoder = BufferEncoder::new(ComplicatedType::HEADER_SIZE, None);
            value0.encode(&mut buffer_encoder, 0);
            buffer_encoder.finalize()
        };
        println!("{}", hex::encode(&encoded_value));
        let mut buffer_decoder = BufferDecoder::new(encoded_value.as_slice());
        let mut value1 = Default::default();
        ComplicatedType::decode_body(&mut buffer_decoder, 0, &mut value1);
        assert_eq!(value0, value1);
    }
}
