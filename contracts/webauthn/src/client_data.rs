//! Strict, deterministic parser for the WebAuthn `clientDataJSON` object.
//!
//! The strict entrypoint must decide policy on JSON *semantics*, not on byte ranges chosen by the
//! caller: a signed object may carry duplicate or decoy `type`, `challenge`, and `origin` members
//! that make a selected slice look correct while the object means something else. This parser
//! accepts a single well-formed JSON object, rejects duplicate member names, decodes string
//! escapes, and returns the decoded members so callers compare values instead of slices.
//!
//! The accepted profile is a subset of RFC 8259 with fixed size and depth limits, so the work done
//! for a given input is bounded and independent of caller-supplied offsets.

use alloc::{string::String, vec::Vec};

/// Maximum accepted `clientDataJSON` size, in bytes.
///
/// Conforming clients emit a few hundred bytes; the limit leaves room for long origins and client
/// specific members while bounding parsing work.
pub const MAX_CLIENT_DATA_LEN: usize = 2048;

/// Maximum accepted JSON nesting depth. The top-level object itself counts as depth 1.
pub const MAX_CLIENT_DATA_DEPTH: usize = 8;

/// The only `type` value accepted for an authentication assertion.
pub const CLIENT_DATA_TYPE_GET: &str = "webauthn.get";

/// Reason a `clientDataJSON` object was rejected by the strict profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDataError {
    /// Input is larger than [`MAX_CLIENT_DATA_LEN`].
    TooLarge,
    /// Input is not valid UTF-8.
    NotUtf8,
    /// Input is not exactly one well-formed JSON object.
    Malformed,
    /// Nesting is deeper than [`MAX_CLIENT_DATA_DEPTH`].
    TooDeep,
    /// The same member name appears more than once in one object.
    DuplicateMember,
    /// A required member is missing.
    MissingMember,
    /// A known member has the wrong JSON type.
    WrongType,
}

/// The `clientDataJSON` members interpreted by the strict profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientData {
    /// Decoded `type` member.
    pub ty: String,
    /// Decoded `challenge` member, still base64url encoded as the client wrote it.
    pub challenge: String,
    /// Decoded `origin` member.
    pub origin: String,
    /// Decoded `crossOrigin` member; `false` when the member is absent.
    pub cross_origin: bool,
}

/// Parses `clientDataJSON` under the strict profile.
///
/// Unknown members are allowed, as the specification requires, but they are fully parsed and count
/// against the size and depth limits. Duplicate member names are rejected in every object, so each
/// of `type`, `challenge`, and `origin` appears exactly once when this returns.
pub fn parse_client_data(input: &[u8]) -> Result<ClientData, ClientDataError> {
    if input.len() > MAX_CLIENT_DATA_LEN {
        return Err(ClientDataError::TooLarge);
    }
    core::str::from_utf8(input).map_err(|_| ClientDataError::NotUtf8)?;

    let mut parser = Parser { input, pos: 0 };
    parser.skip_whitespace();
    let members = parser.parse_object(1)?;
    parser.skip_whitespace();
    if parser.pos != input.len() {
        return Err(ClientDataError::Malformed);
    }

    let mut ty = None;
    let mut challenge = None;
    let mut origin = None;
    let mut cross_origin = false;

    for (name, value) in members {
        match name.as_str() {
            "type" => ty = Some(into_string(value)?),
            "challenge" => challenge = Some(into_string(value)?),
            "origin" => origin = Some(into_string(value)?),
            "crossOrigin" => {
                cross_origin = match value {
                    Value::Bool(flag) => flag,
                    _ => return Err(ClientDataError::WrongType),
                }
            }
            _ => {}
        }
    }

    Ok(ClientData {
        ty: ty.ok_or(ClientDataError::MissingMember)?,
        challenge: challenge.ok_or(ClientDataError::MissingMember)?,
        origin: origin.ok_or(ClientDataError::MissingMember)?,
        cross_origin,
    })
}

/// A parsed JSON value. Contents of nested objects and arrays are validated but not retained,
/// because the strict profile never compares them.
enum Value {
    Str(String),
    Bool(bool),
    Null,
    Number,
    Array,
    Object,
}

fn into_string(value: Value) -> Result<String, ClientDataError> {
    match value {
        Value::Str(string) => Ok(string),
        _ => Err(ClientDataError::WrongType),
    }
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn expect(&mut self, byte: u8) -> Result<(), ClientDataError> {
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            Err(ClientDataError::Malformed)
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Vec<(String, Value)>, ClientDataError> {
        if depth > MAX_CLIENT_DATA_DEPTH {
            return Err(ClientDataError::TooDeep);
        }
        self.expect(b'{')?;

        let mut members: Vec<(String, Value)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(members);
        }

        loop {
            self.skip_whitespace();
            let name = self.parse_string()?;
            if members.iter().any(|(existing, _)| *existing == name) {
                return Err(ClientDataError::DuplicateMember);
            }
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.parse_value(depth)?;
            members.push((name, value));

            self.skip_whitespace();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(members),
                _ => return Err(ClientDataError::Malformed),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<(), ClientDataError> {
        if depth > MAX_CLIENT_DATA_DEPTH {
            return Err(ClientDataError::TooDeep);
        }
        self.expect(b'[')?;

        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(());
        }

        loop {
            self.skip_whitespace();
            self.parse_value(depth)?;
            self.skip_whitespace();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(()),
                _ => return Err(ClientDataError::Malformed),
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value, ClientDataError> {
        match self.peek().ok_or(ClientDataError::Malformed)? {
            b'"' => Ok(Value::Str(self.parse_string()?)),
            b'{' => {
                self.parse_object(depth + 1)?;
                Ok(Value::Object)
            }
            b'[' => {
                self.parse_array(depth + 1)?;
                Ok(Value::Array)
            }
            b't' => {
                self.expect_literal(b"true")?;
                Ok(Value::Bool(true))
            }
            b'f' => {
                self.expect_literal(b"false")?;
                Ok(Value::Bool(false))
            }
            b'n' => {
                self.expect_literal(b"null")?;
                Ok(Value::Null)
            }
            b'-' | b'0'..=b'9' => {
                self.parse_number()?;
                Ok(Value::Number)
            }
            _ => Err(ClientDataError::Malformed),
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), ClientDataError> {
        for byte in literal {
            self.expect(*byte)?;
        }
        Ok(())
    }

    fn parse_string(&mut self) -> Result<String, ClientDataError> {
        self.expect(b'"')?;

        let mut decoded: Vec<u8> = Vec::new();
        loop {
            match self.bump().ok_or(ClientDataError::Malformed)? {
                b'"' => {
                    // The whole input was checked as UTF-8 and strings are only split at ASCII
                    // delimiters, so the decoded bytes are valid UTF-8.
                    return String::from_utf8(decoded).map_err(|_| ClientDataError::NotUtf8);
                }
                b'\\' => {
                    let escape = self.bump().ok_or(ClientDataError::Malformed)?;
                    let unescaped = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.parse_unicode_escape()?,
                        _ => return Err(ClientDataError::Malformed),
                    };
                    let mut buffer = [0u8; 4];
                    decoded.extend_from_slice(unescaped.encode_utf8(&mut buffer).as_bytes());
                }
                byte if byte < 0x20 => return Err(ClientDataError::Malformed),
                byte => decoded.push(byte),
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, ClientDataError> {
        let first = self.parse_hex4()?;
        match first {
            // A high surrogate is only valid when a low surrogate escape follows.
            0xD800..=0xDBFF => {
                self.expect(b'\\')?;
                self.expect(b'u')?;
                let second = self.parse_hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(ClientDataError::Malformed);
                }
                let code =
                    0x10000 + ((u32::from(first - 0xD800) << 10) | u32::from(second - 0xDC00));
                char::from_u32(code).ok_or(ClientDataError::Malformed)
            }
            0xDC00..=0xDFFF => Err(ClientDataError::Malformed),
            _ => char::from_u32(u32::from(first)).ok_or(ClientDataError::Malformed),
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, ClientDataError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let byte = self.bump().ok_or(ClientDataError::Malformed)?;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(ClientDataError::Malformed),
            };
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<(), ClientDataError> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }

        // A leading zero may not be followed by more digits, so `01` is rejected.
        match self.bump().ok_or(ClientDataError::Malformed)? {
            b'0' => {}
            b'1'..=b'9' => self.skip_digits(),
            _ => return Err(ClientDataError::Malformed),
        }

        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.expect_digit()?;
            self.skip_digits();
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.expect_digit()?;
            self.skip_digits();
        }

        Ok(())
    }

    fn expect_digit(&mut self) -> Result<(), ClientDataError> {
        match self.peek() {
            Some(b'0'..=b'9') => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(ClientDataError::Malformed),
        }
    }

    fn skip_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::ToString, vec};

    const ORIGIN: &str = "http://localhost:3005";
    const CHALLENGE: &str = "9jEFijuhEWrM4SOW-tChJbUEHEP44Vcjs";

    fn canonical() -> String {
        format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\"origin\":\"{ORIGIN}\"}}"
        )
    }

    fn parse(json: &str) -> Result<ClientData, ClientDataError> {
        parse_client_data(json.as_bytes())
    }

    #[test]
    fn canonical_object_is_accepted() {
        let data = parse(&canonical()).unwrap();

        assert_eq!(data.ty, CLIENT_DATA_TYPE_GET);
        assert_eq!(data.challenge, CHALLENGE);
        assert_eq!(data.origin, ORIGIN);
        assert!(!data.cross_origin);
    }

    #[test]
    fn conforming_client_vectors_are_accepted() {
        // Chrome appends `crossOrigin` and a free-form member; Safari emits the members alone.
        let chrome = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\"origin\":\"{ORIGIN}\",\
             \"crossOrigin\":false,\"other_keys_can_be_added_here\":\"do not compare clientDataJSON \
             against a template. See https://goo.gl/yabPex\"}}"
        );
        let with_whitespace = format!(
            "{{ \"type\" : \"webauthn.get\" ,\n\t\"challenge\" : \"{CHALLENGE}\" , \
             \"origin\" : \"{ORIGIN}\" }}"
        );
        // Member order carries no meaning in JSON, so a reordered object must still be accepted.
        let reordered = format!(
            "{{\"origin\":\"{ORIGIN}\",\"challenge\":\"{CHALLENGE}\",\"type\":\"webauthn.get\"}}"
        );

        for json in [chrome, with_whitespace, reordered] {
            let data = parse(&json).unwrap();
            assert_eq!(data.ty, CLIENT_DATA_TYPE_GET);
            assert_eq!(data.challenge, CHALLENGE);
            assert_eq!(data.origin, ORIGIN);
            assert!(!data.cross_origin);
        }
    }

    #[test]
    fn duplicate_members_are_rejected() {
        let duplicates = [
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\
                 \"origin\":\"{ORIGIN}\",\"origin\":\"https://evil.example\"}}"
            ),
            format!(
                "{{\"origin\":\"https://evil.example\",\"type\":\"webauthn.get\",\
                 \"challenge\":\"{CHALLENGE}\",\"origin\":\"{ORIGIN}\"}}"
            ),
            format!(
                "{{\"type\":\"webauthn.create\",\"type\":\"webauthn.get\",\
                 \"challenge\":\"{CHALLENGE}\",\"origin\":\"{ORIGIN}\"}}"
            ),
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"decoy\",\
                 \"challenge\":\"{CHALLENGE}\",\"origin\":\"{ORIGIN}\"}}"
            ),
            // An escaped member name decodes to the same name, so it is a duplicate too.
            format!(
                "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\
                 \"origin\":\"{ORIGIN}\",\"\\u006frigin\":\"https://evil.example\"}}"
            ),
        ];

        for json in duplicates {
            assert_eq!(
                parse(&json),
                Err(ClientDataError::DuplicateMember),
                "{json}"
            );
        }
    }

    #[test]
    fn decoy_members_do_not_change_the_decoded_values() {
        // The decoys live inside an unknown member's string value and inside a nested object, both
        // of which the old index-based check could be pointed at.
        let json = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\"origin\":\"{ORIGIN}\",\
             \"decoy\":\"\\\"origin\\\":\\\"https://evil.example\\\"\",\
             \"nested\":{{\"origin\":\"https://evil.example\",\"type\":\"webauthn.create\"}}}}"
        );

        let data = parse(&json).unwrap();

        assert_eq!(data.ty, CLIENT_DATA_TYPE_GET);
        assert_eq!(data.origin, ORIGIN);
    }

    #[test]
    fn escaped_values_are_decoded_before_comparison() {
        let json = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\
             \"origin\":\"http:\\/\\/localhost:\\u0033005\"}}"
        );

        assert_eq!(parse(&json).unwrap().origin, ORIGIN);

        // Surrogate pairs decode to a single scalar value.
        let json = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\
             \"origin\":\"https://\\ud83d\\ude00.example\"}}"
        );

        assert_eq!(parse(&json).unwrap().origin, "https://\u{1f600}.example");
    }

    #[test]
    fn malformed_objects_are_rejected() {
        let malformed = [
            // Not a single top-level object.
            "".to_string(),
            "\"webauthn.get\"".to_string(),
            format!("[{}]", canonical()),
            format!("{} ", canonical()).replace('}', ""),
            // Trailing content after the object, where a second decoy object could hide.
            format!("{}{}", canonical(), canonical()),
            format!("{}!", canonical()),
            // Structural errors.
            canonical().replace("\"origin\"", "origin"),
            canonical().replace("\"type\":", "\"type\"="),
            canonical().replace("}", ",}"),
            format!("{{,{}", &canonical()[1..]),
            // Unterminated string and a raw control character inside one.
            canonical().replace("localhost:3005\"}", "localhost:3005}"),
            canonical().replace("http://", "http\n://"),
            // Invalid escapes.
            canonical().replace("http://", "\\x"),
            canonical().replace("http", "\\u00"),
            canonical().replace("http", "\\uZZZZ"),
            // Lone surrogates.
            canonical().replace("http", "\\ud83d"),
            canonical().replace("http", "\\ude00"),
            // Non-canonical numbers.
            canonical().replace("}", ",\"count\":01}"),
            canonical().replace("}", ",\"count\":1.}"),
            canonical().replace("}", ",\"count\":1e}"),
            canonical().replace("}", ",\"count\":+1}"),
            // Javascript literals that are not JSON.
            canonical().replace("}", ",\"count\":NaN}"),
            canonical().replace("}", ",\"crossOrigin\":False}"),
        ];

        for json in malformed {
            assert!(
                matches!(
                    parse(&json),
                    Err(ClientDataError::Malformed | ClientDataError::TooDeep)
                ),
                "expected rejection of {json}"
            );
        }
    }

    #[test]
    fn missing_or_mistyped_members_are_rejected() {
        assert_eq!(
            parse(&canonical().replace("\"origin\"", "\"Origin\"")),
            Err(ClientDataError::MissingMember)
        );
        assert_eq!(
            parse(&canonical().replace("\"webauthn.get\"", "null")),
            Err(ClientDataError::WrongType)
        );
        assert_eq!(
            parse(&canonical().replace(&format!("\"{CHALLENGE}\""), "1234")),
            Err(ClientDataError::WrongType)
        );
        assert_eq!(
            parse(&canonical().replace("}", ",\"crossOrigin\":\"false\"}")),
            Err(ClientDataError::WrongType)
        );
        assert_eq!(parse("{}"), Err(ClientDataError::MissingMember));
    }

    #[test]
    fn cross_origin_member_is_decoded() {
        assert!(
            parse(&canonical().replace("}", ",\"crossOrigin\":true}"))
                .unwrap()
                .cross_origin
        );
        assert!(
            !parse(&canonical().replace("}", ",\"crossOrigin\":false}"))
                .unwrap()
                .cross_origin
        );
    }

    #[test]
    fn size_and_depth_limits_are_enforced() {
        let padding = "a".repeat(MAX_CLIENT_DATA_LEN);
        let oversized = canonical().replace("}", &format!(",\"pad\":\"{padding}\"}}"));
        assert_eq!(parse(&oversized), Err(ClientDataError::TooLarge));

        let at_limit = format!(
            "{}{}",
            canonical(),
            " ".repeat(MAX_CLIENT_DATA_LEN - canonical().len())
        );
        assert_eq!(at_limit.len(), MAX_CLIENT_DATA_LEN);
        assert!(parse(&at_limit).is_ok());

        // The top-level object is depth 1, so MAX_CLIENT_DATA_DEPTH - 1 nested arrays fit.
        let nesting = |levels: usize| {
            let value = format!("{}{}", "[".repeat(levels), "]".repeat(levels));
            canonical().replace("}", &format!(",\"nested\":{value}}}"))
        };
        assert!(parse(&nesting(MAX_CLIENT_DATA_DEPTH - 1)).is_ok());
        assert_eq!(
            parse(&nesting(MAX_CLIENT_DATA_DEPTH)),
            Err(ClientDataError::TooDeep)
        );
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut bytes = canonical().into_bytes();
        let position = bytes.iter().position(|byte| *byte == b'h').unwrap();
        bytes.splice(position..position, vec![0xff]);

        assert_eq!(parse_client_data(&bytes), Err(ClientDataError::NotUtf8));
    }
}
