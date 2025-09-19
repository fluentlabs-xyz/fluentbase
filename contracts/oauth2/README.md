# OAUTH2

OAuth2 verification helper contract. Verifies OAuth2/OpenID Connect tokens or proofs via host-backed cryptography and
policy checks.

- Entrypoint: main_entry. Decodes request, verifies signatures/claims against configured issuers, emits result.
- Input: token blob and parameters (issuer, audience, nonce, etc.); encoding defined by this crate.
- Output: compact success/failure code and selected claims if verification succeeds.
- Gas/fuel: Bounded primarily by signature verification; charged by host with final settlement.

Note: Requires host-provided trust anchors (JWKS) and time/clock access consistent with verification semantics.
