# scripts/xp — the same stand at an arbitrary N

`devnet/local-dpos-smoke` is hardwired to four validators plus a full-node. That
is exactly the size at which the committee floor is indistinguishable from any
other failure: one exit or one tombstone takes the selection-visible population
to three. Anything about committee size needs N as a knob, so this directory
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
```

The image is the ordinary `fluent-dpos-smoke:local` — build it once (`make up`,
or any smoke case) before the first bring-up; `genN.py` only writes compose files.

Host ports: validator-0 RPC `28000+N`, full-node `29000+N`, validator `i`'s reth
registry `26000+N*100+i`, its commonware registry `27000+N*100+i`.
