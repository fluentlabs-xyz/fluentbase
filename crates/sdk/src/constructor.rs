use crate::leb128;
use alloc::vec::Vec;

/// Encodes a custom section for a WebAssembly (WASM) module.
fn encode_wasm_custom_section(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut section = Vec::new();

    let name_length = leb128::write::unsigned(name.len() as u64);
    let content_length =
        leb128::write::unsigned((name_length.len() + name.len() + payload.len()) as u64);

    section.push(0x00); // Section ID 0x00 for custom sections.
    section.extend(content_length); // Size of all following data in a section encoded as leb128
    section.extend(name_length); // Size of utf-8 encoded name, encoded as leb128
    section.extend_from_slice(name.as_bytes()); // Name encoded as utf-8
    section.extend(payload); // Provided payload
    section
}

/// Packs constructor parameters into a WASM custom section labeled "input".
///
/// The resulting section can be appended to the end of an existing WASM binary.
/// During deployment, these parameters will be accessible via the
/// `sdk.input()` function, allowing the "deploy" function to retrieve
/// necessary data.
pub fn encode_constructor_params(payload: &[u8]) -> Vec<u8> {
    encode_wasm_custom_section("input", payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_wasm_input_section() {
        let payload = vec![1, 2, 3, 4];
        let result = encode_constructor_params(&payload);

        let expected_prefix = vec![0x00, 0x0A, 0x05, b'i', b'n', b'p', b'u', b't']; // header for "input" with payload
        let expected_result = [expected_prefix, payload.clone()].concat();

        assert_eq!(result, expected_result);
    }
}
