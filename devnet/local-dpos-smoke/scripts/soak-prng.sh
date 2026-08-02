#!/usr/bin/env bash
# Seeded integer PRNG for the production soak (sourced by case-soak.sh).
#
# A counter-mode sha256 stream: byte-stable on any host / bash version, unlike
# bash `$RANDOM` (16-bit, RNG-impl-dependent across bash builds — NOT portably
# reproducible, so it is rejected here). The whole point of the soak is that the
# logged SOAK_SEED replays the identical INTENT schedule (plan §1.1); that
# guarantee rests on this stream being a pure function of (SOAK_SEED, counter).
#
# IMPORTANT — the draw functions return their value in the GLOBAL `SOAK_RAND`, NOT
# on stdout, and MUST be called DIRECTLY (never as `$(next_u32)`): a command
# substitution runs in a SUBSHELL, so the `SOAK_PRNG_CTR` increment would be lost
# in the parent and the stream would never advance (every draw identical). Call
# `next_u32; x=$SOAK_RAND` (not `x=$(next_u32)`).
#
# Contract: callers draw in a FIXED order, one `next_u32`/`next_mod` per logical
# draw, and MUST NOT insert conditional draws (plan §1.1: stream position at round
# R is always 4·R), or replay diverges.

# NOTE: do NOT `: "${SOAK_SEED:=0}"` here — sourcing this BEFORE case-soak.sh's
# fresh-seed default would pin an unseeded run to seed 0 (every run identical).
# The seed is defaulted by the orchestrator; next_u32 falls back to 0 only if a
# standalone sourcer never sets it (keeps it set -u-safe).
: "${SOAK_PRNG_CTR:=0}"
SOAK_RAND=0

# Advance the stream by one; result (decimal, [0,2^32)) lands in SOAK_RAND. The
# sha256sum subshell is fine — it does not mutate state; only the counter does,
# and that happens in THIS shell because the function is called directly.
next_u32() {
    local s
    s=$(printf '%s' "${SOAK_SEED:-0}:${SOAK_PRNG_CTR}" | sha256sum | cut -c1-8)
    SOAK_PRNG_CTR=$((SOAK_PRNG_CTR + 1))
    SOAK_RAND=$(( 16#$s ))
}

# Uniform draw in [0, $1) → SOAK_RAND (consumes exactly one stream position).
next_mod() {
    next_u32
    local m="$1"
    if (( m > 0 )); then SOAK_RAND=$(( SOAK_RAND % m )); else SOAK_RAND=0; fi
}
