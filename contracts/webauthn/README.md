# WebAuthn Precompile Documentation

This document provides information about the WebAuthn precompile contract, which offers functionality for P256 signature verification and WebAuthn assertion validation.

## Overview

The WebAuthn precompile provides two main functions:

1. P256 signature verification
2. WebAuthn assertion validation

These functions are accessible through their respective function selectors.

## Function Selectors

| Function | Selector |
|----------|----------|
| `verifyP256Signature` | `0xc358910e` |
| `verifyWebauthn` | `0xbccf2ab7` |

## Function Signatures

### verifyP256Signature

```solidity
function verifyP256Signature(
    bytes32 messageHash,
    bytes32 r,
    bytes32 s,
    bytes32 x,
    bytes32 y,
    bool malleabilityCheck
) external view returns (bool)
```

Verifies a P256 (secp256r1) ECDSA signature.

**Parameters:**

- `messageHash`: The 32-byte hash of the message that was signed
- `r`: The r component of the signature (32 bytes)
- `s`: The s component of the signature (32 bytes)
- `x`: The x coordinate of the public key (32 bytes)
- `y`: The y coordinate of the public key (32 bytes)
- `malleabilityCheck`: If true, rejects signatures with high s values (s > n/2). You should set this to true to prevent signature malleability attacks.

**Returns:**

- `bool`: True if the signature is valid, false otherwise

### verifyWebauthn

```solidity
function verifyWebauthn(
    bytes calldata challenge,
    bytes calldata authenticatorData,
    bool requireUserVerification,
    bytes calldata clientDataJSON,
    uint32 challengeLocation,
    uint32 responseTypeLocation,
    bytes32 r,
    bytes32 s,
    bytes32 x,
    bytes32 y
) external view returns (bool)
```

Verifies a WebAuthn assertion.

**Parameters:**

- `challenge`: The original challenge sent to the authenticator
- `authenticatorData`: The authenticator data returned from the device
- `requireUserVerification`: If true, requires the User Verified (UV) flag to be set. You should set this to true if you want to ensure that the user has been verified.
- `clientDataJSON`: The client data JSON returned from the authenticator
- `challengeLocation`: The byte position where the challenge property starts in the clientDataJSON (as a uint32)
- `responseTypeLocation`: The byte position where the type property starts in the clientDataJSON (as a uint32)
- `r`: The r component of the signature (32 bytes)
- `s`: The s component of the signature (32 bytes)
- `x`: The x coordinate of the public key (32 bytes)
- `y`: The y coordinate of the public key (32 bytes)

**Returns:**

- `bool`: True if the WebAuthn assertion is valid, false otherwise

## Special Features

### Dummy Signature Mode

If `responseTypeLocation` is set to `uint32.max` in the `verifyWebauthn` function, the precompile will skip the WebAuthn-specific checks and only verify the signature. This is useful for testing or for cases where the WebAuthn checks have already been performed elsewhere.

### Signature Malleability Check

The P256 signature verification includes an optional malleability check. When enabled, it rejects signatures with high s values (s > n/2) to prevent signature malleability attacks. This follows the standard practice in ECDSA implementations.

## Implementation Notes

- The precompile uses the P256 (secp256r1) elliptic curve, which is different from Ethereum's default secp256k1 curve.
- The WebAuthn verification follows the WebAuthn Level 2 specification.
- The precompile returns a 32-byte value with either 0 or 1 at the end to indicate the verification result.
