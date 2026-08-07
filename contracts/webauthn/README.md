# WebAuthn Contract for Fluentbase

A WebAuthn assertion verification contract for blockchain authentication.

## Overview

This contract implements WebAuthn verification for blockchain applications, allowing users to authenticate using their device's secure hardware (like TouchID, FaceID, or security keys) instead of traditional passwords or private keys.

The implementation follows the cryptographic assertion-verification subset of the [W3C WebAuthn Level 2](https://www.w3.org/TR/webauthn-2/) specification and is based on reference implementations from:

- [Solady](https://github.com/vectorized/solady/blob/main/src/utils/WebAuthn.sol)
- [Daimo](https://github.com/daimo-eth/p256-verifier/blob/master/src/WebAuthn.sol)
- [Coinbase](https://github.com/base-org/webauthn-sol/blob/main/src/WebAuthn.sol)

This contract exposes both the original assertion primitive and a stricter selector that enforces caller-controlled RP ID hash and origin policy. Callers must still bind the assertion to their own account, challenge lifecycle, and credential state.

## Features

- **Secure Authentication**: Verify WebAuthn assertions using the secp256r1 (P-256) elliptic curve
- **Selective Verification**: Implements critical security checks while omitting unnecessary validations for blockchain use
- **User Verification Control**: Optional enforcement of user verification (biometric/PIN)
- **Backup State Validation**: Checks for proper backup eligibility and state flags
- **Efficient Implementation**: Optimized for blockchain execution

## Interface

### Function Selector

The contract exposes two entrypoint selectors. The legacy selector is `0x94516dde`, derived from:

```
keccak256("verify(bytes,bool,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
```

The strict selector is `0x42520fdd`, derived from:

```
keccak256("verifyStrict(bytes,bool,bytes32,bytes,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
```

### Legacy Input Parameters

The legacy selector takes the following parameters:

1. `challenge` (bytes): The original challenge sent to the authenticator
2. `require_user_verification` (bool): Whether to require the User Verified (UV) flag
3. `auth` (WebAuthnAuth struct): The WebAuthn authentication data containing:
   - `authenticator_data` (bytes): Data from the authenticator including RP ID hash, flags, and counter
   - `client_data_json` (bytes): Client data JSON containing type, challenge, and origin
   - `challenge_index` (uint256): Start index of "challenge" in client_data_json
   - `type_index` (uint256): Start index of "type" in client_data_json
   - `r` (bytes32): The r component of the signature
   - `s` (bytes32): The s component of the signature
4. `x` (uint256): The x coordinate of the public key
5. `y` (uint256): The y coordinate of the public key

### Strict Input Parameters

The strict selector takes the following parameters:

1. `challenge` (bytes): The original challenge sent to the authenticator
2. `require_user_verification` (bool): Whether to require the User Verified (UV) flag
3. `expected_rp_id_hash` (bytes32): SHA-256 hash of the expected RP ID, compared with the first 32 bytes of `authenticator_data`
4. `expected_origin` (bytes): Expected origin, compared with the decoded `origin` member of `client_data_json`
5. `auth` (WebAuthnAuth struct): The WebAuthn authentication data. The strict selector parses `client_data_json` and ignores `challenge_index` and `type_index`
6. `x` (uint256): The x coordinate of the public key
7. `y` (uint256): The y coordinate of the public key

### Return Value

The contract returns a 32-byte value:

- If verification succeeds: A 32-byte value with the last byte set to 1 (true)
- If verification fails: A 32-byte value of all zeros (false)

## Verification Process

The contract performs the following verification steps:

1. **Client Data Verification**:
   - The legacy selector verifies the type is "webauthn.get" and the challenge at the supplied indexes
   - The strict selector parses `client_data_json` and compares the decoded `type`, `challenge`, and `origin` members

2. **Authenticator Data Validation**:
   - The strict selector verifies the RP ID hash matches `expected_rp_id_hash`
   - Checks the User Present (UP) flag is set
   - Verifies the User Verified (UV) flag if required
   - Validates backup state consistency

3. **Signature Verification**:
   - Computes the message hash: SHA-256(authenticator_data || SHA-256(client_data_json))
   - Verifies the signature using the secp256r1 precompile

### Strict Client Data Profile

The strict selector never interprets caller-supplied offsets into `client_data_json`, because a
signed object can contain duplicate or decoy `type`, `challenge`, and `origin` members that make a
selected byte range look correct while the JSON means something else. Instead it parses the object
under a deterministic profile and compares decoded values:

- Input must be exactly one well-formed JSON object of at most 2048 bytes, valid UTF-8, nested at
  most 8 levels deep. Trailing content after the object is rejected.
- Duplicate member names are rejected in every object, so `type`, `challenge`, and `origin` each
  appear exactly once and each must be a string.
- String escapes, including `\uXXXX` and surrogate pairs, are decoded before comparison; lone
  surrogates, invalid escapes, and raw control characters are rejected.
- Unknown members are allowed, as the specification requires, but they are fully parsed and count
  against the size and depth limits.
- The decoded `challenge` must equal the canonical base64url encoding of the expected challenge,
  which rejects padded or non-URL-safe encodings.
- `crossOrigin`, when present, must be a boolean, and a cross-origin assertion is rejected because
  the policy names a single expected origin.

## Standards Compliance and Caller Policy

The W3C assertion verification procedure includes checks that require Relying Party state and policy. This contract verifies:

- `clientDataJSON.type == "webauthn.get"`, at the supplied `type_index` for the legacy selector and from the parsed object for the strict selector
- the expected challenge, at the supplied `challenge_index` for the legacy selector and from the parsed object for the strict selector
- User Present (UP), optional User Verified (UV), and backup-state flag consistency
- the P-256 signature over `authenticator_data || SHA-256(client_data_json)`

The caller is still responsible for enforcing:

- Expected origin and RP ID policy when using the legacy selector. The strict selector enforces a single expected origin and RP ID hash supplied by the caller.
- Credential binding. The supplied public key coordinates must be the registered credential public key for the authenticated account.
- Challenge freshness and single use. A valid old assertion must not be replayable.
- Signature counter policy, if the application relies on clone detection.
- Credential ID lookup, allow-list policy, and account ownership.
- Client extension outputs and token binding, and cross-origin policy when using the legacy selector. The strict selector rejects assertions marked `crossOrigin`.

The `client_data_json`, `authenticator_data`, indexes, and public key are user-supplied inputs. They must not be treated as trusted application data merely because this contract returns `true`; the application must compare policy-critical fields against values it controls.

## Security Considerations

This implementation deliberately omits some full Relying Party validations:

- Multiple allowed origins or richer origin parsing
- Extension outputs
- Signature counter
- Attestation objects

The legacy selector matches `type` and `challenge` as substrings at caller-supplied indexes and does not parse `client_data_json`, so a signed object with duplicate or decoy members can satisfy the selected bytes while meaning something else. It should be treated as a cryptographic assertion primitive, with the calling application enforcing client data policy itself. Prefer the strict selector when the caller wants the contract to enforce client data semantics, RP ID hash, and origin before returning signature success.

## License

This project is part of the Fluentbase ecosystem.
