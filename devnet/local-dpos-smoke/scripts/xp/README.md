# scripts/xp — the same stand at an arbitrary N

`devnet/local-dpos-smoke` is hardwired to five validators plus a full-node —
four before 2026-09-07, when `smoke-byzantine` needed a legal committee to
survive its own tombstone. One exit or one tombstone there now leaves exactly
`MIN_COMMITTEE_LENGTH`, so the stand cannot reach the floor at all: the second
one would, but no smoke case takes two. Anything about committee size needs N as a knob, so this directory
generates the same two-phase stand for any N, and carries the scenarios
`.dpos-study/EXPERIMENTS.md` part 5 was run with.

Everything generated — compose files, overlays, logs, the anchor — goes to
`out/`, which is gitignored.

| file | what it does |
|---|---|
| `genN.py N [byz]` | writes `out/xp<N>.yml` (phase 1: sequencer + cert-upstream followers) and `out/xp<N>.dpos.yml` (phase 2: `--dpos`). Its own docker project `xp<N>`, its own `172.<20+N>.0.0/24`. `--metrics` is on for every validator |
| `bringupN.sh N` | brings `xp<N>` up through the sequencer→DPoS migration: phase 1 to activation, a graceful flush with an exit=0 gate, phase 2, then waits for finalization above the anchor |
| `statusN.sh N` | head / finalized / hash / container state for every node |
| `byz_overlay.sh N i,j` | writes an overlay setting `FLUENT_DPOS_BYZANTINE=equivocate` on the listed validators |
| `inc.py N <byz\|-> <label>` | turns on equivocation, watches what the network does, then tries to bring everything back (E1/E3/E4) |
| `e2exit.sh N i` | E2: `undelegate(validator, self_stake)` from validator `i`'s owner key — the non-byzantine path |
| `e2watch.sh N` | watches the stand after `e2exit.sh` |
| `exits.py N [order]` | E5: validators leave one at a time; after each exit it waits for an epoch boundary and checks whether the chain is alive |
| `engine.py`, `build.py` | engine-API client (HS256 JWT, no dependencies) and block building over `fcuV3 → getPayloadV5 → newPayloadV4`. JWT path from `XP_JWT_PATH`, default `out/jwt.hex` |
| `floor_halt_case.py N <exit\|decay\|byz>` | the committee-floor case (R-111/R-112): drives the population below `MIN_COMMITTEE_LENGTH` and requires the commit to be REFUSED — `commitEpochCommittee(epoch E+3) did not succeed` carrying `CommitteeTooSmall(V, 4)` (`0x0a87ec8d`), no finalized block at or past the boundary, and `nextEpochToCommit()` still `E+3`. `XP_CONTRACTS_DIR` brings the stand up against another artefact directory. `--reuse` / `--keep-up` |
| `metrics_dupcheck.py --stand N` | no duplicate `name{labels}` series on either registry of any node, and the `reth_dpos_executor_*` families are actually exported (R-085 / the reth half of R-070). `--urls URL …` for arbitrary endpoints |
| `agreement_check.py [--stand N]` | every value declared on BOTH sides of the node/contract boundary (R-113..R-119): ABI signatures against `consts.rs` AND the shipped rWasm blob, the numeric literals, the namespace, and — with a stand — dispatch, committee order and the epoch/commit-horizon formulas. Needs `STAKING_CONTRACT_SRC` pointing at the staking contract checkout's `contracts/staking/src`; there is no default |

## The byzantine-overlay threshold

`genN.py` and `byz_overlay.sh` REFUSE at `N < 5`. Not for convenience: on a
committee of four, an equivocation tombstone leaves three selection-visible
validators, so what the run then exercises is the committee-floor path
(R-111/R-112) and not equivocation. If that IS the point — for instance to
re-play `EXPERIMENTS.md` E1 against the carry-over — set `XP_ALLOW_SMALL_BYZ=1`.

## Running it

```
make xp-up     N=6      # generate + bring up
make xp-e5     N=6      # E5 (honest exits) against a stand that is already up
make xp-e2     N=6      # E2 (one honest full exit) + the watch
make xp-byz    N=6      # E1 (equivocation); needs N>=5 unless XP_ALLOW_SMALL_BYZ=1
make xp-status N=6
make xp-down   N=6

make xp-floor   N=4 XP_MODE=byz    # committee-floor refusal (exit | decay | byz)
make xp-metrics N=4                # both registries of every node on the stand
make xp-agree                      # node/contract agreement, offline half
make xp-agree   XP_LIVE=1 N=4      # ...plus the live half against stand xp4
```

`xp-agree` needs `STAKING_CONTRACT_SRC` in the environment either way.

The image is the ordinary `fluent-dpos-smoke:local` — build it once (`make up`,
or any smoke case) before the first bring-up; `genN.py` only writes compose files.

Host ports: validator-0 RPC `28000+N`, full-node `29000+N`, validator `i`'s reth
registry `26000+N*100+i`, its commonware registry `27000+N*100+i`.
