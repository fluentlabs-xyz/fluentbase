# WebAuthn Contract for Fluentbase

A WebAuthn assertion verification contract for blockchain authentication.

## Overview

This contract implements WebAuthn verification for blockchain applications, allowing users to authenticate using their device's secure hardware (like TouchID, FaceID, or security keys) instead of traditional passwords or private keys.

The implementation follows the cryptographic assertion-verification subset of the [W3C WebAuthn Level 2](https://www.w3.org/TR/webauthn-2/) specification and is based on reference implementations from:

- [Solady](https://github.com/vectorized/solady/blob/main/src/utils/WebAuthn.sol)
- [Daimo](https://github.com/daimo-eth/p256-verifier/blob/master/src/WebAuthn.sol)
- [Coinbase](https://github.com/base-org/webauthn-sol/blob/main/src/WebAuthn.sol)

This contract is not a complete WebAuthn Relying Party implementation by itself. Callers must bind the assertion to their own account, origin/RP policy, challenge lifecycle, and credential state.

## Features

- **Secure Authentication**: Verify WebAuthn assertions using the secp256r1 (P-256) elliptic curve
- **Selective Verification**: Implements critical security checks while omitting unnecessary validations for blockchain use
- **User Verification Control**: Optional enforcement of user verification (biometric/PIN)
- **Backup State Validation**: Checks for proper backup eligibility and state flags
- **Efficient Implementation**: Optimized for blockchain execution

## Interface

### Function Selector

The contract exposes a single entry point with function selector `0x94516dde`, derived from:

```
keccak256("verify(bytes,bool,(bytes,bytes,uint256,uint256,bytes32,bytes32),uint256,uint256)")
```

### Input Parameters

The function takes the following parameters:

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
   - Checks the User Present (UP) flag is set
   - Verifies the User Verified (UV) flag if required
   - Validates backup state consistency

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

- Expected origin and RP ID policy. In particular, verify the RP ID hash in `authenticator_data` against the caller's expected RP ID hash.
- Credential binding. The supplied public key coordinates must be the registered credential public key for the authenticated account.
- Challenge freshness and single use. A valid old assertion must not be replayable.
- Signature counter policy, if the application relies on clone detection.
- Credential ID lookup, allow-list policy, and account ownership.
- Client extension outputs, token binding, and cross-origin policy if these are relevant to the application.

The `client_data_json`, `authenticator_data`, indexes, and public key are user-supplied inputs. They must not be treated as trusted application data merely because this contract returns `true`; the application must compare policy-critical fields against values it controls.

## Security Considerations

This implementation deliberately omits some full Relying Party validations from the current ABI:

- Origin and RP ID validation (delegated to authenticator)
- Extension outputs
- Signature counter
- Attestation objects

These omissions optimize gas usage for the current selector, but they mean the selector should be treated as a cryptographic assertion primitive. A future strict selector should take caller-controlled policy inputs such as expected origin, expected RP ID hash, credential ID/public key binding, and counter policy, then reject mismatches inside the contract.

## License

This project is part of the Fluentbase ecosystem.
