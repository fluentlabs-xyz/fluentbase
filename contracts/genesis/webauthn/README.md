# WebAuthn Contract for Fluentbase

A WebAuthn verification contract for blockchain authentication, enabling secure, passwordless authentication using the W3C WebAuthn standard.

## Overview

This contract implements WebAuthn verification for blockchain applications, allowing users to authenticate using their device's secure hardware (like TouchID, FaceID, or security keys) instead of traditional passwords or private keys.

The implementation follows the [W3C WebAuthn Level 2](https://www.w3.org/TR/webauthn-2/) specification and is based on reference implementations from:

- [Solady](https://github.com/vectorized/solady/blob/main/src/utils/WebAuthn.sol)
- [Daimo](https://github.com/daimo-eth/p256-verifier/blob/master/src/WebAuthn.sol)
- [Coinbase](https://github.com/base-org/webauthn-sol/blob/main/src/WebAuthn.sol)

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

## Security Considerations

This implementation deliberately omits certain WebAuthn verifications that are less relevant in a blockchain context:

- Origin and RP ID validation (delegated to authenticator)
- Credential backup state
- Extension outputs
- Signature counter
- Attestation objects

These omissions optimize gas usage while maintaining the security properties essential for blockchain authentication.

## License

This project is part of the Fluentbase ecosystem.
