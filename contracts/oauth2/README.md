# OAUTH2

OAuth2 verification helper contract with ephemeral session support. Verifies OAuth2/OpenID Connect tokens and manages temporary session keys for improved UX.

## Features

- **Direct JWT Verification**: Verify OAuth2 JWT tokens with signature and claims validation
- **Ephemeral Sessions**: Create temporary session keys after OAuth verification (like Sui zkLogin)
- **Multi-Provider**: Google, Apple, Microsoft support with real JWKS keys
- **Standards Compliant**: OpenID Connect, RFC 7519 (JWT), RFC 7515 (JWS)

## Functions

### 1. verifyToken

```solidity
function verifyToken(
    string token,
    string issuer,
    string audience,
    string nonce
) returns (bool valid, string subject, string email)
```

Direct JWT verification - validates token cryptographically and returns user identity.

### 2. createSession (NEW!)

```solidity
function createSession(
    string token,
    string issuer,
    string audience,
    string nonce,
    uint256 durationSeconds
) returns (
    bytes privateKey,
    bytes32 publicKeyX,
    bytes32 publicKeyY,
    uint256 expiresAt,
    string subject
)
```

Creates an ephemeral session key after verifying OAuth token. Session key is valid for specified duration.

### 3. verifySession (NEW!)

```solidity
function verifySession(
    bytes message,
    bytes signature,
    bytes32 publicKeyX,
    bytes32 publicKeyY,
    uint256 expiresAt
) returns (bool valid)
```

Verifies a signature from ephemeral session key. Checks expiration before signature verification.

## Architecture

- Entrypoint: main_entry routes to verifyToken, createSession, or verifySession based on function selector
- Session keys: Regular ECDSA keypairs with expiration metadata (not cryptographically different from permanent keys)
- Expiration: Checked as application policy before cryptographic verification
- Gas: Direct verification ~170k, Session creation ~180k, Session verification ~10-20k

## Security

- JWT signatures verified via RSA modexp precompile (RS256, RS384)
- Claims validated: issuer, audience, expiration, nonce
- Clock skew tolerance: 60 seconds
- Session expiration: Enforced on-chain before signature verification
- Fail-secure: Any verification failure rejects transaction

See E2E_FLOW.md for complete usage examples.
