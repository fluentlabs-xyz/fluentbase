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

The strict selector is `0xd6b45308`, derived from:

```
keccak256("verifyStrict(bytes,bool,bytes32,bytes,uint256,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
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
4. `expected_origin` (bytes): Expected origin bytes, compared with `client_data_json` at `origin_index`
5. `origin_index` (uint256): Start index of `"origin":"..."` in `client_data_json`
6. `auth` (WebAuthnAuth struct): The WebAuthn authentication data
7. `x` (uint256): The x coordinate of the public key
8. `y` (uint256): The y coordinate of the public key

### Return Value

The contract returns a 32-byte value:

- If verification succeeds: A 32-byte value with the last byte set to 1 (true)
- If verification fails: A 32-byte value of all zeros (false)

## Verification Process

The contract performs the following verification steps:

1. **Client Data Verification**:
   - Verifies the type is "webauthn.get"
   - Confirms the challenge matches the expected value

2. **Authenticator Data Validation**:
   - The strict selector verifies the RP ID hash matches `expected_rp_id_hash`
   - Checks the User Present (UP) flag is set
   - Verifies the User Verified (UV) flag if required
   - Validates backup state consistency
   - The strict selector verifies the origin matches `expected_origin` at `origin_index`

3. **Signature Verification**:
   - Computes the message hash: SHA-256(authenticator_data || SHA-256(client_data_json))
   - Verifies the signature using the secp256r1 precompile

## Standards Compliance and Caller Policy

The W3C assertion verification procedure includes checks that require Relying Party state and policy. This contract verifies:

- `clientDataJSON.type == "webauthn.get"` at the supplied `type_index`
- the expected challenge at the supplied `challenge_index`
- User Present (UP), optional User Verified (UV), and backup-state flag consistency
- the P-256 signature over `authenticator_data || SHA-256(client_data_json)`

The caller is still responsible for enforcing:

- Expected origin and RP ID policy when using the legacy selector. The strict selector enforces a single expected origin and RP ID hash supplied by the caller.
- Credential binding. The supplied public key coordinates must be the registered credential public key for the authenticated account.
- Challenge freshness and single use. A valid old assertion must not be replayable.
- Signature counter policy, if the application relies on clone detection.
- Credential ID lookup, allow-list policy, and account ownership.
- Client extension outputs, token binding, and cross-origin policy if these are relevant to the application.

The `client_data_json`, `authenticator_data`, indexes, and public key are user-supplied inputs. They must not be treated as trusted application data merely because this contract returns `true`; the application must compare policy-critical fields against values it controls.

## Security Considerations

This implementation deliberately omits some full Relying Party validations:

- Multiple allowed origins or richer origin parsing
- Extension outputs
- Signature counter
- Attestation objects

The legacy selector should be treated as a cryptographic assertion primitive. Prefer the strict selector when the caller wants the contract to reject RP ID hash or origin mismatches before returning signature success.

## License

This project is part of the Fluentbase ecosystem.
