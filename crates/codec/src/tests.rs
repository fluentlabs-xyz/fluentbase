use super::{BufferDecoder, BufferEncoder, Encoder};
use alloy_primitives::Bytes;
use hashbrown::{HashMap, HashSet};

#[test]
fn test_vec() {
    let values = vec![0, 1, 2, 3];
    let result = {
        let mut buffer_encoder = BufferEncoder::new(12, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    Vec::<i32>::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_bytes() {
    let values = Bytes::from_static("Hello, World".as_bytes());
    let result = {
        let mut buffer_encoder = BufferEncoder::new(Bytes::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    Bytes::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_nested_vec() {
    let values = vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]];
    let result = {
        let mut buffer_encoder = BufferEncoder::new(12, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    Vec::<Vec<i32>>::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_empty_vec() {
    let values: Vec<u32> = vec![];
    let result = {
        let mut buffer_encoder = BufferEncoder::new(12, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    Vec::<u32>::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_map() {
    let mut values = HashMap::new();
    values.insert(100, 20);
    values.insert(3, 5);
    values.insert(1000, 60);
    let result = {
        let mut buffer_encoder = BufferEncoder::new(20, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    HashMap::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_set() {
    let values = HashSet::from([1, 2, 3]);
    let result = {
        let mut buffer_encoder = BufferEncoder::new(HashSet::<i32>::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    HashSet::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_set_is_sorted() {
    let result1 = {
        let values = HashSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut buffer_encoder = BufferEncoder::new(HashSet::<i32>::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    let result2 = {
        let values = HashSet::from([8, 3, 2, 4, 5, 9, 7, 1, 6]);
        let mut buffer_encoder = BufferEncoder::new(HashSet::<i32>::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    assert_eq!(result1, result2);
}

#[test]
fn test_nested_map() {
    let mut values = HashMap::new();
    values.insert(100, HashMap::from([(1, 2), (3, 4)]));
    values.insert(3, HashMap::new());
    values.insert(1000, HashMap::from([(7, 8), (9, 4)]));
    let result = {
        let mut buffer_encoder =
            BufferEncoder::new(HashMap::<i32, HashMap<i32, i32>>::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    HashMap::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_vector_of_maps() {
    let mut values = Vec::new();
    values.push(HashMap::from([(1, 2), (3, 4)]));
    values.push(HashMap::new());
    values.push(HashMap::from([(7, 8), (9, 4)]));
    let result = {
        let mut buffer_encoder = BufferEncoder::new(Vec::<HashMap<i32, i32>>::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    Vec::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_map_of_vectors() {
    let mut values = HashMap::new();
    values.insert(vec![0, 1, 2], vec![3, 4, 5]);
    values.insert(vec![3, 1, 2], vec![3, 4, 5]);
    values.insert(vec![0, 1, 6], vec![3, 4, 5]);
    let result = {
        let mut buffer_encoder =
            BufferEncoder::new(HashMap::<Vec<i32>, Vec<i32>>::HEADER_SIZE, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    HashMap::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_static_array() {
    let values = [1, 2, 3];
    let result = {
        let mut buffer_encoder = BufferEncoder::new(3 * 4, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    <[i32; 3]>::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_empty_static_array() {
    let values: [u8; 0] = [];
    let result = {
        let mut buffer_encoder = BufferEncoder::new(0, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    <[u8; 0]>::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_static_array_of_arrays() {
    let values = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
    let result = {
        let mut buffer_encoder = BufferEncoder::new(3 * 4 * 3, None);
        values.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut values2 = Default::default();
    <[[i32; 3]; 3]>::decode_body(&mut buffer_decoder, 0, &mut values2);
    assert_eq!(values, values2);
}

#[test]
fn test_option() {
    let value1 = Some(0x7bu32);
    let value2 = None;
    let result = {
        let mut buffer_encoder = BufferEncoder::new(5 + 5, None);
        value1.encode(&mut buffer_encoder, 0);
        value2.encode(&mut buffer_encoder, 5);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut decoded1 = Default::default();
    let mut decoded2 = Default::default();
    Option::<u32>::decode_header(&mut buffer_decoder, 0, &mut decoded1);
    Option::<u32>::decode_header(&mut buffer_decoder, 5, &mut decoded2);
    assert_eq!(value1, decoded1);
    assert_eq!(value2, decoded2);
}

#[test]
fn test_option_non_primitive() {
    let value = Some(vec![1, 2, 3]);
    let result = {
        let mut buffer_encoder = BufferEncoder::new(value.header_size(), None);
        value.encode(&mut buffer_encoder, 0);
        buffer_encoder.finalize()
    };
    println!("{}", hex::encode(&result));
    let mut buffer_decoder = BufferDecoder::new(result.as_slice());
    let mut decoded_value = Default::default();
    Option::<Vec<u32>>::decode_body(&mut buffer_decoder, 0, &mut decoded_value);
    assert_eq!(value, decoded_value);
}

pub trait Messenger {
    fn send(&self, msg: &str);
}

pub struct LimitTracker<'a, T: Messenger> {
    messenger: &'a T,
    value: usize,
    max: usize,
}

impl<'a, T> LimitTracker<'a, T>
where
    T: Messenger,
{
    pub fn new(messenger: &'a T, max: usize) -> LimitTracker<'a, T> {
        LimitTracker {
            messenger,
            value: 0,
            max,
        }
    }

    pub fn set_value(&mut self, value: usize) {
        self.value = value;

        let percentage_of_max = self.value as f64 / self.max as f64;

        if percentage_of_max >= 1.0 {
            self.messenger.send("Error: You are over your quota!");
        } else if percentage_of_max >= 0.9 {
            self.messenger
                .send("Urgent warning: You've used up over 90% of your quota!");
        } else if percentage_of_max >= 0.75 {
            self.messenger
                .send("Warning: You've used up over 75% of your quota!");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct MockMessenger {
        sent_messages: RefCell<Vec<String>>,
    }

    impl MockMessenger {
        fn new() -> MockMessenger {
            MockMessenger {
                ..Default::default()
            }
        }
    }

    impl Messenger for MockMessenger {
        fn send(&self, message: &str) {
            let mut one_borrow = self.sent_messages.borrow_mut();
            let mut two_borrow = self.sent_messages.borrow_mut();

            one_borrow.push(String::from(message));
            two_borrow.push(String::from(message));
        }
    }

    #[test]
    fn it_sends_an_over_75_percent_warning_message() {
        let mock_messenger = MockMessenger::new();
        let mut limit_tracker = LimitTracker::new(&mock_messenger, 100);

        limit_tracker.set_value(80);

        assert_eq!(mock_messenger.sent_messages.borrow().len(), 1);
    }
}
