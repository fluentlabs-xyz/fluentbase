use hex::FromHexError;
use rlp::{Decodable, DecoderError, Encodable};
use serde::{
    de::{Error, Unexpected, Visitor},
    Deserialize,
    Deserializer,
    Serialize,
    Serializer,
};
use std::fmt;
use thiserror::Error;

/// Raw bytes wrapper
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Bytes(pub Vec<u8>);

impl<T: Into<Vec<u8>>> From<T> for Bytes {
    fn from(data: T) -> Self {
        Bytes(data.into())
    }
}

impl Serialize for Bytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut serialized = "0x".to_owned();
        serialized.push_str(&hex::encode(&self.0));
        serializer.serialize_str(serialized.as_ref())
    }
}

impl<'a> Deserialize<'a> for Bytes {
    fn deserialize<D>(deserializer: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'a>,
    {
        deserializer.deserialize_identifier(BytesVisitor)
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let serialized = format!("0x{}", hex::encode(&self.0));
        f.debug_tuple("Bytes").field(&serialized).finish()
    }
}

impl Encodable for Bytes {
    fn rlp_append(&self, s: &mut rlp::RlpStream) {
        // println!("IS LIST(Encodable): {:?}", self.rlp_bytes());
        // println!("IS LIST(Encodable): {:?}", self.0.to_vec());
        s.append(&self.rlp_bytes());
    }
}

impl Decodable for Bytes {
    fn decode(d: &rlp::Rlp) -> Result<Self, DecoderError> {
        // println!("IS LIST(Decodable): {:?}", d.li(0));
        Ok(Bytes(d.as_raw().to_vec()))
    }
}

/// Encode hex with 0x prefix
pub(crate) fn hex_encode<T: AsRef<[u8]>>(data: T) -> String {
    format!("0x{}", hex::encode(data))
}

/// An error from a byte utils operation.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ByteUtilsError {
    #[error("Hex string starts with {first_two}, expected 0x")]
    WrongPrefix { first_two: String },

    #[error("Unable to decode hex string {data} due to {source}")]
    HexDecode { source: FromHexError, data: String },

    #[error("Hex string is '{data}', expected to start with 0x")]
    NoPrefix { data: String },
}

/// Decode hex with 0x prefix
pub(crate) fn hex_decode(data: &str) -> Result<Vec<u8>, ByteUtilsError> {
    let first_two = data.get(..2).ok_or_else(|| ByteUtilsError::NoPrefix {
        data: data.to_string(),
    })?;

    if first_two != "0x" {
        return Err(ByteUtilsError::WrongPrefix {
            first_two: first_two.to_string(),
        });
    }

    let post_prefix = data.get(2..).unwrap_or("");

    hex::decode(post_prefix).map_err(|e| ByteUtilsError::HexDecode {
        source: e,
        data: data.to_string(),
    })
}

pub(crate) fn se_hex<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&hex_encode(value))
}

pub fn de_hex_to_vec_u8<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let result: String = Deserialize::deserialize(deserializer)?;
    hex_decode(&result).map_err(serde::de::Error::custom)
}

pub fn de_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let result: String = Deserialize::deserialize(deserializer)?;
    let result = result.trim_start_matches("0x");
    u64::from_str_radix(result, 16).map_err(serde::de::Error::custom)
}

struct BytesVisitor;

impl<'a> Visitor<'a> for BytesVisitor {
    type Value = Bytes;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a 0x-prefixed hex-encoded vector of bytes")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        if let Some(value) = value.strip_prefix("0x") {
            let bytes =
                hex::decode(value).map_err(|e| Error::custom(format!("Invalid hex: {}", e)))?;
            Ok(Bytes(bytes))
        } else {
            Err(Error::invalid_value(Unexpected::Str(value), &"0x prefix"))
        }
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: Error,
    {
        self.visit_str(value.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize() {
        assert_eq!(
            serde_json::from_str::<Bytes>(r#""0x00""#).unwrap(),
            Bytes(vec![0x00])
        );
        assert_eq!(
            serde_json::from_str::<Bytes>(r#""0x0123456789AaBbCcDdEeFf""#).unwrap(),
            Bytes(vec![
                0x01, 0x23, 0x45, 0x67, 0x89, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF
            ])
        );
        assert_eq!(
            serde_json::from_str::<Bytes>(r#""0x""#).unwrap(),
            Bytes(vec![])
        );

        assert!(serde_json::from_str::<Bytes>("0").is_err(), "Not a string");
        assert!(
            serde_json::from_str::<Bytes>(r#""""#).is_err(),
            "Empty string"
        );
        assert!(
            serde_json::from_str::<Bytes>(r#""0xZZ""#).is_err(),
            "Invalid hex"
        );
        assert!(
            serde_json::from_str::<Bytes>(r#""deadbeef""#).is_err(),
            "Missing 0x prefix"
        );
        assert!(
            serde_json::from_str::<Bytes>(r#""数字""#).is_err(),
            "Non-ASCII"
        );
        assert!(
            serde_json::from_str::<Bytes>(r#""0x数字""#).is_err(),
            "Non-ASCII"
        );
    }
}
