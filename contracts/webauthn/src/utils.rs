extern crate alloc;

use alloc::string::String;
use base64::{engine::general_purpose, Engine as _};

/// Convert a slice to a fixed size array
pub fn array_from_slice(slice: &[u8]) -> [u8; 32] {
    let mut array = [0u8; 32];
    array.copy_from_slice(slice);
    array
}

/// Compare two byte arrays
/// Returns:
///   -1 if a < b
///    0 if a == b
///    1 if a > b
pub fn compare_bytes(a: &[u8], b: &[u8]) -> i32 {
    let len_a = a.len();
    let len_b = b.len();

    match len_a.cmp(&len_b) {
        core::cmp::Ordering::Less => return -1,
        core::cmp::Ordering::Greater => return 1,
        core::cmp::Ordering::Equal => {}
    }

    for (byte_a, byte_b) in a.iter().zip(b.iter()) {
        match byte_a.cmp(byte_b) {
            core::cmp::Ordering::Less => return -1,
            core::cmp::Ordering::Greater => return 1,
            core::cmp::Ordering::Equal => {}
        }
    }

    0
}

/// Base64URL encode function
pub fn base64url_encode(input: &[u8]) -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(input)
}

/// Check if a substring exists at a specific position in a string
pub fn contains_at(substr: &[u8], full_str: &[u8], location: usize) -> bool {
    let substr_len = substr.len();

    if location
        .checked_add(substr_len).is_none_or(|end| end > full_str.len())
    {
        return false;
    }

    let slice = &full_str[location..location + substr_len];
    slice == substr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_bytes() {
        // Equal arrays
        assert_eq!(compare_bytes(&[1, 2, 3], &[1, 2, 3]), 0);

        // Different lengths
        assert_eq!(compare_bytes(&[1, 2], &[1, 2, 3]), -1);
        assert_eq!(compare_bytes(&[1, 2, 3], &[1, 2]), 1);

        // Same length, different content
        assert_eq!(compare_bytes(&[1, 2, 3], &[1, 2, 4]), -1);
        assert_eq!(compare_bytes(&[1, 3, 2], &[1, 2, 3]), 1);

        // Empty arrays
        assert_eq!(compare_bytes(&[], &[]), 0);
    }

    #[test]
    fn test_base64url_encode() {
        // Test standard values
        assert_eq!(base64url_encode(b""), "");
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
        assert_eq!(base64url_encode(b"foob"), "Zm9vYg");
        assert_eq!(base64url_encode(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_encode(b"foobar"), "Zm9vYmFy");

        // Test URL safety - no padding and URL-safe characters
        assert_eq!(base64url_encode(b"hello+world/"), "aGVsbG8rd29ybGQv");
        assert_eq!(base64url_encode(b"\xFF\xEF"), "_-8");
    }

    #[test]
    fn test_contains_at() {
        let full_str = b"Hello, world!";

        // Positive tests
        assert!(contains_at(b"Hello", full_str, 0));
        assert!(contains_at(b"world!", full_str, 7));
        assert!(contains_at(b"!", full_str, 12));
        assert!(contains_at(b"", full_str, 5)); // Empty string is always contained

        // Negative tests
        assert!(!contains_at(b"hello", full_str, 0)); // Case sensitive
        assert!(!contains_at(b"Hello", full_str, 1)); // Wrong position

        // Boundary tests
        assert!(!contains_at(b"d!", full_str, 12)); // Exceeds bounds
        assert!(!contains_at(b"Hello", full_str, 9)); // Not enough space

        // Edge cases
        assert!(!contains_at(b"x", &[], 0)); // Empty string doesn't contain non-empty
        assert!(contains_at(b"", &[], 0)); // Empty string contains empty
    }
}
