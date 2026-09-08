//! BLS12-381 MinSig verification, inlined.
//!
//! This is the port of the former external `BLS12381Verifier` Solidity
//! predeploy (`solidity-contracts@f641789f:contracts/libraries/BLS12381Verifier.sol`).
//! It was reached through an address held in this contract's own storage and
//! moved by a governance setter; a substituted verifier accepts a forged
//! proof-of-possession, and a forged PoP forges a whole committee's quorum
//! (`pk' = pk_x - sum(pk_i)`). There is no address left to substitute: the only
//! outbound calls are to the fixed EIP-2537 precompile addresses, which carry no
//! setter.
//!
//! What is inlined is byte assembly and the calls. The arithmetic stays where it
//! was: `0x02` SHA-256, `0x05` MODEXP, `0x0b` G1ADD, `0x0f` PAIRING,
//! `0x10` MAP_FP_TO_G1.
//!
//! Two properties the Solidity carried that must survive any later edit here:
//!
//! * The namespace length prefix is ONE byte and lengths `>= 0x80` revert. The
//!   node writes a full LEB128 varint (`commonware utils::union_unique`); the
//!   two agree only while every namespace is under 128 bytes. Dropping the
//!   check does not make them agree — it makes them hash different messages,
//!   silently, on the day a namespace grows.
//! * `compress_g*_unchecked` proves nothing on its own. It checks neither
//!   on-curve, nor subgroup, nor the EIP-2537 zero padding, nor `x < p`, so two
//!   different uncompressed inputs can compress to the same reference. What
//!   binds a compressed key to its owner is that the SAME uncompressed bytes
//!   then go through PAIRING, which rejects all three deviations. A caller that
//!   compresses and skips the verify has no binding at all.

use crate::{consts::*, util::revert};
use alloc::vec::Vec;
use fluentbase_sdk::{address, hex, Address, Bytes, ExitCode, SharedAPI};

// The EIP-2537 (and SHA-256/MODEXP) precompiles, at the addresses
// `fluentbase_types::genesis` seats them on and `EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES`
// routes to the system runtime. Fixed by the fork, not by this contract's state.
const SHA256: Address = address!("0x0000000000000000000000000000000000000002");
const MODEXP: Address = address!("0x0000000000000000000000000000000000000005");
const G1ADD: Address = address!("0x000000000000000000000000000000000000000b");
const PAIRING: Address = address!("0x000000000000000000000000000000000000000f");
const MAP_FP_TO_G1: Address = address!("0x0000000000000000000000000000000000000010");

/// BLS12-381 base field prime `p`, 48 bytes big-endian.
const P: [u8; 48] =
    hex!("1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab");

/// `(p-1)/2`, 48 bytes big-endian. The y-sign rule is `y > (p-1)/2`, a 384-bit
/// unsigned compare — big-endian byte order compares the same way, so the
/// Solidity's hi/lo split is not needed here. The comparison is STRICT: at
/// exactly `(p-1)/2` the sign bit stays clear.
const HALF: [u8; 48] =
    hex!("0d0088f51cbff34d258dd3db21a5d66bb23ba5c279c2895fb39869507b587b120f55ffff58a9ffffdcff7fffffffd555");

/// Negated G2 generator in EIP-2537 form (256 B). Protocol constant: the second
/// factor of `e(sig, -G2gen) * e(H, pk) == 1`.
const NEG_G2_GENERATOR: [u8; 256] = hex!(
    "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8"
    "0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e"
    "000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa"
    "0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed"
);

/// EIP-2537 G1 width: `pad16 ‖ x(48) ‖ pad16 ‖ y(48)`.
const G1_UNCOMPRESSED_LENGTH: usize = 128;
/// EIP-2537 G2 width: `pad16 ‖ x.c0 ‖ pad16 ‖ x.c1 ‖ pad16 ‖ y.c0 ‖ pad16 ‖ y.c1`.
const G2_UNCOMPRESSED_LENGTH: usize = 256;
/// One field element, big-endian.
const FP_LENGTH: usize = 48;
/// `expand_message_xmd(SHA-256)` block size, and therefore the `Z` prefix length.
const XMD_BLOCK_LENGTH: usize = 64;

/// `staticcall` a precompile and demand an exact output width.
///
/// Any deviation is `PrecompileFailed`: a short MODEXP answer would shift the
/// field element inside the 64-byte MAP input, and a short MAP or G1ADD answer
/// would shift the 384-byte per-pair boundary inside the PAIRING input. Failing
/// closed here is what keeps a truncated answer from being read as a different,
/// well-formed point.
fn call_precompile<SDK: SharedAPI>(
    sdk: &mut SDK,
    target: Address,
    input: &[u8],
    output_length: usize,
) -> Result<Bytes, ExitCode> {
    let result = sdk.static_call(target, input, None);
    if !result.status.is_ok() || result.data.len() != output_length {
        return revert(sdk, ERR_BLS_PRECOMPILE_FAILED);
    }
    Ok(result.data)
}

fn sha256<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<[u8; 32], ExitCode> {
    let output = call_precompile(sdk, SHA256, input, 32)?;
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&output);
    Ok(digest)
}

fn xor32(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for ((slot, left), right) in out.iter_mut().zip(a.iter()).zip(b.iter()) {
        *slot = left ^ right;
    }
    out
}

/// `union_unique(ns, msg) = I2OSP(len(ns), 1) ‖ ns ‖ msg`.
///
/// One length byte, and `>= 0x80` reverts rather than truncating — see the
/// module note on the LEB128 divergence.
fn union_unique<SDK: SharedAPI>(
    sdk: &mut SDK,
    namespace: &[u8],
    message: &[u8],
) -> Result<Vec<u8>, ExitCode> {
    if namespace.len() >= 0x80 {
        return revert(sdk, ERR_BLS_NAMESPACE_TOO_LONG);
    }
    let mut out = Vec::with_capacity(1 + namespace.len() + message.len());
    out.push(namespace.len() as u8);
    out.extend_from_slice(namespace);
    out.extend_from_slice(message);
    Ok(out)
}

/// `MODEXP(base, 1, p)` — a 64-byte value reduced mod `p`, 48 bytes out.
fn modexp_mod_p<SDK: SharedAPI>(sdk: &mut SDK, base: &[u8]) -> Result<Bytes, ExitCode> {
    let mut input = Vec::with_capacity(96 + XMD_BLOCK_LENGTH + 1 + FP_LENGTH);
    for length in [XMD_BLOCK_LENGTH, 1, FP_LENGTH] {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&(length as u64).to_be_bytes());
        input.extend_from_slice(&word);
    }
    input.extend_from_slice(base);
    input.push(1);
    input.extend_from_slice(&P);
    call_precompile(sdk, MODEXP, &input, FP_LENGTH)
}

/// `MAP_FP_TO_G1(pad16 ‖ fp48)` — 128 B EIP-2537 G1.
///
/// The cofactor clearing RFC 9380 §6.6.3 prescribes happens inside this
/// precompile, not here. Fluent computes it with arkworks
/// (`contracts/bls12381` -> `revm-precompile` with the `blst` feature off)
/// while the node signs with blst; the two were checked to agree on the pinned
/// conformance corpus. Drift fails closed — PAIRING rejects a point off the
/// prime-order subgroup, so `verify` returns false and slashing silently does
/// not fire.
fn map_fp_to_g1<SDK: SharedAPI>(sdk: &mut SDK, fp: &[u8]) -> Result<Bytes, ExitCode> {
    let mut input = [0u8; XMD_BLOCK_LENGTH];
    input[16..].copy_from_slice(fp);
    call_precompile(sdk, MAP_FP_TO_G1, &input, G1_UNCOMPRESSED_LENGTH)
}

fn g1_add<SDK: SharedAPI>(sdk: &mut SDK, p0: &[u8], p1: &[u8]) -> Result<Bytes, ExitCode> {
    let mut input = Vec::with_capacity(2 * G1_UNCOMPRESSED_LENGTH);
    input.extend_from_slice(p0);
    input.extend_from_slice(p1);
    call_precompile(sdk, G1ADD, &input, G1_UNCOMPRESSED_LENGTH)
}

/// RFC 9380 `hash_to_curve`: `expand_message_xmd(SHA-256)` to 128 bytes, two
/// field elements, two maps, one add.
///
/// The long-DST workaround (RFC 9380 §5.3.3, `H("H2C-OVERSIZE-DST-" ‖ DST)`) is
/// deliberately absent — both Fluent DSTs are 43 bytes — and anything past the
/// 255-byte short-DST limit reverts instead of being silently rehashed.
fn hash_to_g1<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8], dst: &[u8]) -> Result<Bytes, ExitCode> {
    if dst.len() > 255 {
        return revert(sdk, ERR_BLS_DST_TOO_LONG);
    }
    // DST' = dst ‖ I2OSP(len(dst), 1)
    let mut dst_prime = Vec::with_capacity(dst.len() + 1);
    dst_prime.extend_from_slice(dst);
    dst_prime.push(dst.len() as u8);

    // msgP = Z(64×0x00) ‖ input ‖ I2OSP(128, 2) ‖ 0x00 ‖ DST'
    let mut msg_prime = Vec::with_capacity(XMD_BLOCK_LENGTH + input.len() + 3 + dst_prime.len());
    msg_prime.extend_from_slice(&[0u8; XMD_BLOCK_LENGTH]);
    msg_prime.extend_from_slice(input);
    msg_prime.extend_from_slice(&[0x00, 0x80, 0x00]);
    msg_prime.extend_from_slice(&dst_prime);

    let b0 = sha256(sdk, &msg_prime)?;
    let block = |sdk: &mut SDK, seed: &[u8; 32], index: u8| -> Result<[u8; 32], ExitCode> {
        let mut buffer = Vec::with_capacity(33 + dst_prime.len());
        buffer.extend_from_slice(seed);
        buffer.push(index);
        buffer.extend_from_slice(&dst_prime);
        sha256(sdk, &buffer)
    };
    let b1 = block(sdk, &b0, 1)?;
    let b2 = block(sdk, &xor32(&b0, &b1), 2)?;
    let b3 = block(sdk, &xor32(&b0, &b2), 3)?;
    let b4 = block(sdk, &xor32(&b0, &b3), 4)?;

    // uniform = b1 ‖ b2 ‖ b3 ‖ b4, split into two 64-byte field inputs.
    let mut uniform = [0u8; 128];
    uniform[..32].copy_from_slice(&b1);
    uniform[32..64].copy_from_slice(&b2);
    uniform[64..96].copy_from_slice(&b3);
    uniform[96..].copy_from_slice(&b4);

    let u0 = modexp_mod_p(sdk, &uniform[..XMD_BLOCK_LENGTH])?;
    let u1 = modexp_mod_p(sdk, &uniform[XMD_BLOCK_LENGTH..])?;
    let p0 = map_fp_to_g1(sdk, &u0)?;
    let p1 = map_fp_to_g1(sdk, &u1)?;
    g1_add(sdk, &p0, &p1)
}

/// EIP-2537 encodes the point at infinity as all zeroes, and PAIRING SKIPS an
/// infinity pair rather than failing on it — so an all-zero point would make the
/// equation hold for free. Checked over every byte, as the Solidity did.
fn reject_infinity<SDK: SharedAPI>(sdk: &mut SDK, point: &[u8]) -> Result<(), ExitCode> {
    if point.iter().any(|byte| *byte != 0) {
        return Ok(());
    }
    revert(sdk, ERR_BLS_INFINITY_POINT)
}

fn fp_is_zero(fp: &[u8]) -> bool {
    fp.iter().all(|byte| *byte == 0)
}

fn fp_greater_half(fp: &[u8]) -> bool {
    fp > &HALF[..]
}

/// Verify one MinSig signature: `e(sig, -G2gen) * e(H, pk) == 1`.
///
/// `dst` is a parameter and not a constant on purpose: the PoP domain and the
/// vote domain must stay separate, or a vote signature becomes usable as a
/// proof-of-possession. The two callers pass `BLS_POP_DST` and `BLS_SIG_DST`.
///
/// Binding the signature and the key to a trust anchor is the CALLER's job. This
/// is the pairing and nothing else.
pub(crate) fn verify<SDK: SharedAPI>(
    sdk: &mut SDK,
    namespace: &[u8],
    message: &[u8],
    dst: &[u8],
    sig_uncompressed: &[u8],
    pk_uncompressed: &[u8],
) -> Result<bool, ExitCode> {
    // Pin exact EIP-2537 widths: any deviation shifts the 384-byte per-pair
    // boundary inside the two-pair PAIRING input below.
    if sig_uncompressed.len() != G1_UNCOMPRESSED_LENGTH
        || pk_uncompressed.len() != G2_UNCOMPRESSED_LENGTH
    {
        return revert(sdk, ERR_BLS_INVALID_POINT_LENGTH);
    }
    reject_infinity(sdk, sig_uncompressed)?;
    reject_infinity(sdk, pk_uncompressed)?;

    let preimage = union_unique(sdk, namespace, message)?;
    let h = hash_to_g1(sdk, &preimage, dst)?;
    reject_infinity(sdk, &h)?;

    // (sig ‖ -G2gen) ‖ (H ‖ pk) = 768 bytes.
    let mut input = Vec::with_capacity(768);
    input.extend_from_slice(sig_uncompressed);
    input.extend_from_slice(&NEG_G2_GENERATOR);
    input.extend_from_slice(&h);
    input.extend_from_slice(pk_uncompressed);

    // A bad point is `false`, never a revert: off-curve, off-subgroup, non-zero
    // EIP-2537 padding and a non-canonical coordinate all arrive here and are
    // all rejected by the precompile. Reverting on a failed pairing would turn
    // "this signature is not valid" into "this transaction is malformed".
    let result = sdk.static_call(PAIRING, &input, None);
    if !result.status.is_ok() || result.data.len() != 32 {
        return Ok(false);
    }
    let mut expected = [0u8; 32];
    expected[31] = 1;
    Ok(result.data.as_ref() == expected)
}

/// Compress a 128 B EIP-2537 G1 to its 48 B zcash form.
///
/// UNCHECKED, and the name is the contract: on-curve and subgroup are left to
/// PAIRING, so the result is a reference to compare against a pre-anchored
/// identity, never evidence that the input was a real point.
pub(crate) fn compress_g1_unchecked<SDK: SharedAPI>(
    sdk: &mut SDK,
    uncompressed: &[u8],
) -> Result<[u8; BLS_SIGNATURE_LENGTH], ExitCode> {
    if uncompressed.len() != G1_UNCOMPRESSED_LENGTH {
        return revert(sdk, ERR_BLS_INVALID_POINT_LENGTH);
    }
    let x = &uncompressed[16..64];
    let y = &uncompressed[80..128];
    // Compressing the infinity encoding would yield a valid-looking 0x80… key,
    // which is exactly the reference an attacker would want to collide with.
    if fp_is_zero(x) && fp_is_zero(y) {
        return revert(sdk, ERR_BLS_INFINITY_POINT);
    }
    let mut out = [0u8; BLS_SIGNATURE_LENGTH];
    out.copy_from_slice(x);
    out[0] |= 0x80 | if fp_greater_half(y) { 0x20 } else { 0x00 };
    Ok(out)
}

/// Compress a 256 B EIP-2537 G2 to its 96 B zcash form.
///
/// The halves SWAP: EIP-2537 orders `x.c0` first, zcash orders `x.c1` first.
/// That single reordering is the difference between a key that matches the one
/// the node registered and one that matches nothing.
pub(crate) fn compress_g2_unchecked<SDK: SharedAPI>(
    sdk: &mut SDK,
    uncompressed: &[u8],
) -> Result<[u8; BLS_PUBKEY_LENGTH], ExitCode> {
    if uncompressed.len() != G2_UNCOMPRESSED_LENGTH {
        return revert(sdk, ERR_BLS_INVALID_POINT_LENGTH);
    }
    let x_c0 = &uncompressed[16..64];
    let x_c1 = &uncompressed[80..128];
    let y_c0 = &uncompressed[144..192];
    let y_c1 = &uncompressed[208..256];
    if fp_is_zero(x_c0) && fp_is_zero(x_c1) && fp_is_zero(y_c0) && fp_is_zero(y_c1) {
        return revert(sdk, ERR_BLS_INFINITY_POINT);
    }
    // Fp2 sign is lexicographic, c1 before c0.
    let sign = fp_greater_half(y_c1) || (fp_is_zero(y_c1) && fp_greater_half(y_c0));
    let mut out = [0u8; BLS_PUBKEY_LENGTH];
    out[..FP_LENGTH].copy_from_slice(x_c1);
    out[FP_LENGTH..].copy_from_slice(x_c0);
    out[0] |= 0x80 | if sign { 0x20 } else { 0x00 };
    Ok(out)
}
