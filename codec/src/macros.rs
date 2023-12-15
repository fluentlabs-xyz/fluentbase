macro_rules! derive_header_size {
    // for an empty input just return 0 size, because there is no fields
    () => (0);
    ($val:ident: $typ:ty) => {
        <$typ as $crate::Encoder<$typ>>::HEADER_SIZE
    };
    ($val_x:ident:$typ_x:ty, $($val_y:ident:$typ_y:ty),+ $(,)?) => {
        derive_header_size!($val_x:$typ_x) + derive_header_size!($($val_y:$typ_y),+)
    };
}
macro_rules! derive_encode {
    // for an empty input just do nothing, because there is no fields
    () => ();
    ($self:expr, $encoder:expr, $field_offset:expr, $val:ident: $typ:ty) => {
        $self.$val.encode($encoder, $field_offset)
    };
    ($self:expr, $encoder:expr, $field_offset:expr, $val_x:ident:$typ_x:ty, $($val_y:ident:$typ_y:ty),+ $(,)?) => {
        derive_encode!($self, $encoder, $field_offset, $val_x:$typ_x);
        $field_offset += derive_header_size!($val_x:$typ_x);
        derive_encode!($self, $encoder, $field_offset, $($val_y:$typ_y),+)
    };
}
macro_rules! derive_decode {
    // for an empty input just do nothing, because there is no fields
    () => ();
    ($self:expr, $decoder:expr, $field_offset:expr, $val:ident: $typ:ty) => {
        <$typ as $crate::Encoder<$typ>>::decode($decoder, $field_offset, &mut $self.$val)
    };
    ($self:expr, $decoder:expr, $field_offset:expr, $val_x:ident:$typ_x:ty, $($val_y:ident:$typ_y:ty),+ $(,)?) => {
        derive_decode!($self, $decoder, $field_offset, $val_x:$typ_x);
        $field_offset += derive_header_size!($val_x:$typ_x);
        derive_decode!($self, $decoder, $field_offset, $($val_y:$typ_y),+)
    };
}
macro_rules! derive_types {
    // for an empty input just do nothing
    ($field_offset:expr,) => {};
    ($field_offset:expr, $val_head:ident: $typ_head:ty, $($val_next:ident:$typ_next:ty,)* $(,)?) => {
        type $val_head = $crate::FieldEncoder<$typ_head, { $field_offset }>;
        derive_types!($field_offset + <$typ_head as $crate::Encoder<$typ_head>>::HEADER_SIZE, $($val_next:$typ_next,)*);
    };
}

#[macro_export]
macro_rules! define_codec_struct {
    (pub struct $struct_type:ident { $($element:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Default, PartialEq)]
        pub struct $struct_type {
            $($element: $ty),*
        }
        impl $crate::Encoder<$struct_type> for $struct_type {
            const HEADER_SIZE: usize = derive_header_size!($($element:$ty),*);
            fn encode(&self, encoder: &mut $crate::BufferEncoder, mut field_offset: usize) {
                derive_encode!(self, encoder, field_offset, $($element:$ty),*);
            }
            fn decode(decoder: &mut $crate::BufferDecoder, mut field_offset: usize, result: &mut $struct_type) {
                derive_decode!(result, decoder, field_offset, $($element:$ty),*);
            }
        }
        impl $struct_type {
            derive_types!(0, $($element:$ty,)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::{BufferDecoder, BufferEncoder, Encoder};
    use hashbrown::HashMap;

    define_codec_struct! {
        pub struct SimpleType {
            a: u64,
            b: u32,
            c: u16,
        }
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
        SimpleType::decode(&mut buffer_decoder, 0, &mut value1);
        assert_eq!(value0, value1);
    }

    define_codec_struct! {
        pub struct ComplicatedType {
            values: Vec<SimpleType>,
            maps: HashMap<u32, ComplicatedType>,
        }
    }

    #[test]
    fn test_decode_specific_field() {
        let value = SimpleType {
            a: 100,
            b: 20,
            c: 3,
        };
        // check offsets
        assert_eq!(SimpleType::a::FIELD_OFFSET, 0);
        assert_eq!(SimpleType::b::FIELD_OFFSET, 8);
        assert_eq!(SimpleType::c::FIELD_OFFSET, 8 + 4);
        // check sizes
        assert_eq!(SimpleType::a::FIELD_SIZE, 8);
        assert_eq!(SimpleType::b::FIELD_SIZE, 4);
        assert_eq!(SimpleType::c::FIELD_SIZE, 2);
        // encode entire struct
        let encoded_value = value.encode_to_vec(0);
        let encoded_value = encoded_value.as_slice();
        // decode only field `a`
        {
            let mut a: u64 = 0;
            SimpleType::a::decode_field_from_slice(
                &encoded_value[SimpleType::a::FIELD_OFFSET
                    ..(SimpleType::a::FIELD_OFFSET + SimpleType::a::FIELD_SIZE)],
                &mut a,
            );
            assert_eq!(a, value.a);
        }
        // decode only field `b`
        {
            let mut b: u32 = 0;
            SimpleType::b::decode_field_from_slice(
                &encoded_value[SimpleType::b::FIELD_OFFSET
                    ..(SimpleType::b::FIELD_OFFSET + SimpleType::b::FIELD_SIZE)],
                &mut b,
            );
            assert_eq!(b, value.b);
        }
        // decode only field `c`
        {
            let mut c: u16 = 0;
            SimpleType::c::decode_field_from_slice(
                &encoded_value[SimpleType::c::FIELD_OFFSET
                    ..(SimpleType::c::FIELD_OFFSET + SimpleType::c::FIELD_SIZE)],
                &mut c,
            );
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
        ComplicatedType::decode(&mut buffer_decoder, 0, &mut value1);
        assert_eq!(value0, value1);
    }
}
