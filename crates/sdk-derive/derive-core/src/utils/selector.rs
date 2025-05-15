use crypto_hashes::{digest::Digest, sha3::Keccak256};
use proc_macro2::Span;
use syn::Error;

pub const SELECTOR_SIZE: usize = 4;
/// Basic type for 4-byte function selector
pub type Selector = [u8; 4];

/// Computes the Keccak256 hash of a signature and returns the first 4 bytes.
#[must_use]
pub fn calculate_keccak256(signature: &str) -> Selector {
    let mut hasher = Keccak256::new();
    hasher.update(signature.as_bytes());
    let mut selector = [0u8; SELECTOR_SIZE];
    selector.copy_from_slice(&hasher.finalize()[..SELECTOR_SIZE]);
    selector
}

/// Parses a hexadecimal string into a 4-byte array selector.
///
/// # Arguments
/// * `hex` - Hex string starting with "0x" followed by 8 hex characters
///
/// # Returns
/// * `Ok(Selector)` - Successfully parsed 4-byte selector
/// * `Err(Error)` - Parse error with detailed message
pub fn parse_hex_string(hex: &str) -> Result<Selector, Error> {
    let hex = hex
        .strip_prefix("0x")
        .ok_or_else(|| Error::new(Span::call_site(), "Hex string must start with '0x'"))?;

    if hex.len() != 8 {
        return Err(Error::new(
            Span::call_site(),
            format!(
                "Invalid hex string length. Expected 8 characters, found {}",
                hex.len()
            ),
        ));
    }

    let bytes = hex::decode(hex)
        .map_err(|e| Error::new(Span::call_site(), format!("Invalid hex string: {e}")))?;

    let mut selector = [0u8; SELECTOR_SIZE];
    selector.copy_from_slice(&bytes);
    Ok(selector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_signature_selector() {
        let signature = "transfer(address,uint256)";
        let selector = calculate_keccak256(signature);
        assert_eq!(selector.len(), SELECTOR_SIZE);
    }

    #[test]
    fn test_parse_hex_selector_valid() {
        let hex = "0x12345678";
        let selector = parse_hex_string(hex).unwrap();
        assert_eq!(selector, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_parse_hex_selector_without_prefix() {
        let hex = "12345678";
        assert!(parse_hex_string(hex).is_err());
    }

    #[test]
    fn test_parse_hex_selector_wrong_length() {
        let hex = "0x123456";
        assert!(parse_hex_string(hex).is_err());
    }

    #[test]
    fn test_parse_hex_selector_invalid_chars() {
        let hex = "0x1234567g";
        assert!(parse_hex_string(hex).is_err());
    }
}
