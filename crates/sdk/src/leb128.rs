pub const CONTINUATION_BIT: u8 = 1 << 7;
pub const SIGN_BIT: u8 = 1 << 6;

#[inline]
pub fn low_bits_of_byte(byte: u8) -> u8 {
    byte & !CONTINUATION_BIT
}

#[inline]
pub fn low_bits_of_u64(val: u64) -> u8 {
    let byte = val & (u8::MAX as u64);
    low_bits_of_byte(byte as u8)
}

/// A module for writing LEB128-encoded signed and unsigned integers.
///
/// Adapted from <https://crates.io/crates/leb128>, with references to the Rust standard
/// library removed, as we do not use the standard library in the SDK crate.
pub mod write {
    use super::{low_bits_of_u64, CONTINUATION_BIT};
    use alloc::vec::Vec;

    pub fn unsigned(mut val: u64) -> Vec<u8> {
        let mut result: Vec<u8> = Vec::new();
        loop {
            let mut byte = low_bits_of_u64(val);
            val >>= 7;
            if val != 0 {
                // More bytes to come, so set the continuation bit.
                byte |= CONTINUATION_BIT;
            }

            result.push(byte);

            if val == 0 {
                return result;
            }
        }
    }

    pub fn signed(mut val: i64) -> Vec<u8> {
        let mut result: Vec<u8> = Vec::new();
        loop {
            let mut byte = val as u8;
            // Keep the sign bit for testing
            val >>= 6;
            let done = val == 0 || val == -1;
            if done {
                byte &= !CONTINUATION_BIT;
            } else {
                // Remove the sign bit
                val >>= 1;
                // More bytes to come, so set the continuation bit.
                byte |= CONTINUATION_BIT;
            }

            result.push(byte);

            if done {
                return result;
            }
        }
    }
}

/// A module for reading LEB128-encoded signed and unsigned integers.
pub mod read {
    use super::{low_bits_of_byte, CONTINUATION_BIT, SIGN_BIT};

    /// An error type for reading LEB128-encoded values.
    #[derive(Debug)]
    pub enum Error {
        /// There was an underlying IO error.
        UnexpectedEof,
        /// The number being read is larger than can be represented.
        Overflow,
    }

    /// Read an unsigned LEB128-encoded number from the `std::io::Read` stream
    /// `r`.
    ///
    /// On success, return the number.
    pub fn unsigned(r: &[u8]) -> Result<u64, Error> {
        let mut result = 0;
        let mut shift = 0;
        let mut pos = 0;

        loop {
            let mut byte = *r.get(pos).ok_or(Error::UnexpectedEof)?;
            pos += 1;

            if shift == 63 && byte != 0x00 && byte != 0x01 {
                while byte & CONTINUATION_BIT != 0 {
                    byte = *r.get(pos).ok_or(Error::UnexpectedEof)?;
                    pos += 1;
                }
                return Err(Error::Overflow);
            }

            let low_bits = low_bits_of_byte(byte) as u64;
            result |= low_bits << shift;

            if byte & CONTINUATION_BIT == 0 {
                return Ok(result);
            }

            shift += 7;
        }
    }

    /// Read a signed LEB128-encoded number from the `std::io::Read` stream `r`.
    ///
    /// On success, return the number.
    pub fn signed(r: &[u8]) -> Result<i64, Error> {
        let mut result = 0;
        let mut shift = 0;
        let size = 64;
        let mut pos = 0;
        let mut byte;

        loop {
            byte = *r.get(pos).ok_or(Error::UnexpectedEof)?;
            pos += 1;

            if shift == 63 && byte != 0x00 && byte != 0x7f {
                while byte & CONTINUATION_BIT != 0 {
                    byte = *r.get(pos).ok_or(Error::UnexpectedEof)?;
                    pos += 1;
                }
                return Err(Error::Overflow);
            }

            let low_bits = low_bits_of_byte(byte) as i64;
            result |= low_bits << shift;
            shift += 7;

            if byte & CONTINUATION_BIT == 0 {
                break;
            }
        }

        if shift < size && (SIGN_BIT & byte) == SIGN_BIT {
            // Sign extend the result.
            result |= !0 << shift;
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dogfood_signed() {
        fn inner(i: i64) {
            let buf = write::signed(i);
            let mut readable = &buf[..];
            let result = read::signed(&mut readable).expect("Should be able to read it back again");
            assert_eq!(i, result);
        }
        for i in -513..513 {
            inner(i);
        }
        inner(i64::MIN);
        inner(i64::MAX);
        inner(9999999);
        inner(-9999999);
    }

    #[test]
    fn dogfood_unsigned() {
        for i in 0..3999 {
            let buf = write::unsigned(i);

            let mut readable = &buf[..];
            let result =
                read::unsigned(&mut readable).expect("Should be able to read it back again");
            assert_eq!(i, result);
        }
    }
}
