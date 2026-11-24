# Nitro Verifier Contract

AWS Nitro Enclaves attestation document verifier for Fluentbase genesis/system-contract usage.

## Overview

The contract accepts a COSE_Sign1-wrapped Nitro attestation document, parses the signed CBOR payload, validates the attestation document shape, validates the AWS Nitro certificate chain against the pinned Nitro root certificate, and verifies the COSE signature with the leaf certificate.

The implementation follows the AWS Nitro Enclaves attestation validation flow:

- [AWS Nitro Enclaves root of trust](https://docs.aws.amazon.com/enclaves/latest/user/verify-root.html)
- [AWS Nitro Enclaves attestation process](https://github.com/aws/aws-nitro-enclaves-nsm-api/blob/main/docs/attestation_process.md)

## What This Verifier Checks

- Input above `NITRO_MAX_INPUT_SIZE` (16 KiB) is rejected before it is parsed.
- The input is exactly the COSE_Sign1 envelope Nitro emits: an untagged array of four items with the protected header `{1: -35}` (ES384), an empty unprotected header, a payload and a 96-byte signature. Anything else is rejected without being handed to a general CBOR parser.
- The payload nests no deeper than an attestation document does (two levels).
- The payload is a Nitro attestation document with required fields.
- The digest is `SHA384`.
- PCR count, PCR indexes, PCR byte lengths, and optional field sizes are within expected bounds.
- The cabundle starts with the pinned AWS Nitro root certificate.
- Certificates parse as DER, are valid at the block timestamp, carry required extensions, and form a valid chain.
- The COSE_Sign1 signature verifies with the attestation leaf certificate.

## Caller Policy

This contract verifies that an attestation document is genuinely signed by the AWS Nitro attestation PKI. That is necessary, but not sufficient, for application-level trust.

Callers must compare policy-critical fields against values they control:

- `nonce`: bind the attestation to a caller-generated challenge and reject replay.
- `pcrs`: require the exact expected enclave measurements.
- `public_key`: require the expected key when using attestation to bootstrap encrypted sessions or account ownership.
- `user_data`: require exact expected application data if the protocol uses it.
- `module_id`: require the expected module identity if relevant.
- `timestamp`: enforce a freshness window appropriate for the protocol.

The attestation document's optional `nonce`, `public_key`, and `user_data` fields are user-controlled by the enclave/protocol. They must not be trusted simply because the attestation signature is valid; callers must compare them to expected values supplied by their own protocol.

## Standards Compliance Notes

The verifier covers the base AWS attestation document validation path and root-of-trust verification. It does not perform application-specific authorization or freshness checks, because those require caller-provided policy. It also does not perform CRL/OCSP revocation checks.

A future strict selector should accept expected nonce, PCR policy, public key, user data, module id, and max age as calldata, then reject any mismatch inside the contract.

## Gas/Fuel

The verifier is an engine-metered system runtime: the execution engine charges every instruction it executes, so the cost scales with the attestation being verified. The entrypoint charges no additional static fuel, and input above `NITRO_MAX_INPUT_SIZE` is rejected before it is parsed.

Every rejection reverts with a `nitro: ...` reason and returns the unused gas, so a caller that relays somebody else's attestation loses only the gas the rejected call used.
