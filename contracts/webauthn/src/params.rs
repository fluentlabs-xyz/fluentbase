extern crate alloc;

use alloc::{vec, vec::Vec};

/// WebAuthn parameters structure
#[derive(Debug, Clone)]
pub struct WebAuthnParams<'a> {
    pub challenge: &'a [u8],
    pub authenticator_data: &'a [u8],
    pub require_user_verification: bool,
    pub client_data_json: &'a [u8],
    pub challenge_location: u32,
    pub response_type_location: u32,
    pub r: [u8; 32],
    pub s: [u8; 32],
    pub x: [u8; 32],
    pub y: [u8; 32],
}

#[allow(dead_code)]
impl WebAuthnParams<'_> {
    /// Encode WebAuthn parameters according to Solidity ABI encoding format
    pub fn encode(&self) -> Vec<u8> {
        let static_size = 320;

        let challenge_offset = static_size;
        let challenge_data_size = 32 + ((self.challenge.len() + 31) / 32) * 32;

        let auth_data_offset = challenge_offset + challenge_data_size;
        let auth_data_size = 32 + ((self.authenticator_data.len() + 31) / 32) * 32;

        let client_data_json_offset = auth_data_offset + auth_data_size;

        let mut result = Vec::with_capacity(
            static_size
                + challenge_data_size
                + auth_data_size
                + 32
                + ((self.client_data_json.len() + 31) / 32) * 32,
        );

        // Add offsets and boolean
        let mut offset_bytes = [0u8; 32];
        offset_bytes[28..32].copy_from_slice(&(challenge_offset as u32).to_be_bytes());
        result.extend_from_slice(&offset_bytes);

        let mut bool_bytes = [0u8; 32];
        bool_bytes[31] = if self.require_user_verification { 1 } else { 0 };
        result.extend_from_slice(&bool_bytes);

        let mut offset_bytes = [0u8; 32];
        offset_bytes[28..32].copy_from_slice(&(auth_data_offset as u32).to_be_bytes());
        result.extend_from_slice(&offset_bytes);

        let mut offset_bytes = [0u8; 32];
        offset_bytes[28..32].copy_from_slice(&(client_data_json_offset as u32).to_be_bytes());
        result.extend_from_slice(&offset_bytes);

        // Add u32 values with padding
        let mut u32_bytes = [0u8; 32];
        u32_bytes[28..32].copy_from_slice(&self.challenge_location.to_be_bytes());
        result.extend_from_slice(&u32_bytes);

        let mut u32_bytes = [0u8; 32];
        u32_bytes[28..32].copy_from_slice(&self.response_type_location.to_be_bytes());
        result.extend_from_slice(&u32_bytes);

        // Add byte arrays
        result.extend_from_slice(&self.r);
        result.extend_from_slice(&self.s);
        result.extend_from_slice(&self.x);
        result.extend_from_slice(&self.y);

        // Add dynamic data with padding
        Self::add_padded_data(&mut result, self.challenge);
        Self::add_padded_data(&mut result, self.authenticator_data);
        Self::add_padded_data(&mut result, self.client_data_json);

        result
    }

    /// Helper method to add length-prefixed data with padding
    fn add_padded_data(result: &mut Vec<u8>, data: &[u8]) {
        let mut length_bytes = [0u8; 32];
        length_bytes[28..32].copy_from_slice(&(data.len() as u32).to_be_bytes());
        result.extend_from_slice(&length_bytes);
        result.extend_from_slice(data);

        let padding_needed = (32 - (data.len() % 32)) % 32;
        if padding_needed > 0 {
            result.extend_from_slice(&vec![0u8; padding_needed]);
        }
    }
}

/// Read an u32 from a big-endian byte slice
pub fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 <= data.len() {
        Some(u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]))
    } else {
        None
    }
}

/// Parse WebAuthn parameters from input data following Solidity ABI encoding format
pub fn parse_webauthn_params(params: &[u8]) -> Option<WebAuthnParams> {
    if params.len() < 320 {
        return None;
    }

    let challenge_offset = read_u32_be(params, 28)? as usize;
    let require_user_verification = params[63] != 0; // last byte of the first 32 bytes
    let auth_data_offset = read_u32_be(params, 92)? as usize;
    let client_data_json_offset = read_u32_be(params, 124)? as usize;

    // Read u32 values from the appropriate offsets
    let challenge_location = read_u32_be(params, 156)?; // 128 + 28
    let response_type_location = read_u32_be(params, 188)?; // 160 + 28

    // Extract byte arrays for cryptographic parameters
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];

    r.copy_from_slice(&params[192..224]);
    s.copy_from_slice(&params[224..256]);
    x.copy_from_slice(&params[256..288]);
    y.copy_from_slice(&params[288..320]);

    if challenge_offset >= params.len()
        || auth_data_offset >= params.len()
        || client_data_json_offset >= params.len()
    {
        return None;
    }

    // Read dynamic data
    let challenge_len = read_u32_be(params, challenge_offset + 28)? as usize;
    let challenge_start = challenge_offset + 32;
    if challenge_start + challenge_len > params.len() {
        return None;
    }
    let challenge = &params[challenge_start..challenge_start + challenge_len];

    let auth_data_len = read_u32_be(params, auth_data_offset + 28)? as usize;
    let auth_data_start = auth_data_offset + 32;
    if auth_data_start + auth_data_len > params.len() {
        return None;
    }
    let authenticator_data = &params[auth_data_start..auth_data_start + auth_data_len];

    let client_data_json_len = read_u32_be(params, client_data_json_offset + 28)? as usize;
    let client_data_json_start = client_data_json_offset + 32;
    if client_data_json_start + client_data_json_len > params.len() {
        return None;
    }
    let client_data_json =
        &params[client_data_json_start..client_data_json_start + client_data_json_len];

    Some(WebAuthnParams {
        challenge,
        authenticator_data,
        require_user_verification,
        client_data_json,
        challenge_location,
        response_type_location,
        r,
        s,
        x,
        y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_u32_be() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A];

        // Valid read
        assert_eq!(read_u32_be(&data, 0), Some(0x12345678));

        // Read at boundary
        assert_eq!(read_u32_be(&data, 1), Some(0x3456789A));

        // Out of bounds
        assert_eq!(read_u32_be(&data, 2), None);
    }

    #[test]
    fn test_webauthn_params_roundtrip() {
        // Create test data
        let challenge = b"Test WebAuthn Challenge";
        let authenticator_data = &[
            0x49, 0x96, 0x0d, 0xe5, 0x88, 0x0e, 0x8c, 0x68, 0x74, 0x34, 0x17, 0x0f, 0x64, 0x76,
            0x60, 0x5b, 0x8f, 0xe4, 0xae, 0xb9, 0xa2, 0x86, 0x32, 0xc7, 0x99, 0x5c, 0xf3, 0xba,
            0x83, 0x1d, 0x97, 0x63, 0x05, 0x00, 0x00, 0x01, 0x01,
        ];
        let require_user_verification = true;
        let client_data_json = b"{\"type\":\"webauthn.get\",\"challenge\":\"VGVzdCBXZWJBdXRobiBDaGFsbGVuZ2U\",\"origin\":\"http://localhost\"}";
        let challenge_location = 23u32;
        let response_type_location = 7u32;

        // Create test signature and key components
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];

        // Fill with test values
        for i in 0..32 {
            r[i] = (i + 1) as u8;
            s[i] = (i + 33) as u8;
            x[i] = (i + 65) as u8;
            y[i] = (i + 97) as u8;
        }

        // Create WebAuthnParams struct
        let params = WebAuthnParams {
            challenge,
            authenticator_data,
            require_user_verification,
            client_data_json,
            challenge_location,
            response_type_location,
            r,
            s,
            x,
            y,
        };

        // Encode parameters using the new method
        let encoded = params.encode();

        // Decode parameters
        let decoded = parse_webauthn_params(&encoded).expect("Failed to decode parameters");

        // Verify round-trip results
        assert_eq!(decoded.challenge, challenge, "Challenge mismatch");
        assert_eq!(
            decoded.authenticator_data, authenticator_data,
            "Authenticator data mismatch"
        );
        assert_eq!(
            decoded.require_user_verification, require_user_verification,
            "User verification flag mismatch"
        );
        assert_eq!(
            decoded.client_data_json, client_data_json,
            "Client data JSON mismatch"
        );
        assert_eq!(
            decoded.challenge_location, challenge_location,
            "Challenge location mismatch"
        );
        assert_eq!(
            decoded.response_type_location, response_type_location,
            "Response type location mismatch"
        );
        assert_eq!(decoded.r, r, "Signature r component mismatch");
        assert_eq!(decoded.s, s, "Signature s component mismatch");
        assert_eq!(decoded.x, x, "Public key x coordinate mismatch");
        assert_eq!(decoded.y, y, "Public key y coordinate mismatch");

        // Double check by encoding the decoded parameters again
        let re_encoded = decoded.encode();

        // Verify that the re-encoded data matches the original encoding
        assert_eq!(encoded, re_encoded, "Re-encoded data mismatch");
    }
}
