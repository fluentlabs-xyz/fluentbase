"""dpos_harness — the DPoS devnet harness framework, and the consumers built on it.

LAYERS, strict downward dependency. An import from a lower layer to a higher one is a build
error to be fixed, not discussed (tests/test_layering.py enforces it):

  cases/   growth, quorum, seed_continuity
  sim/     orchestrator, rounds, dispatch, reconcilers, hostpool, actions, state_ctx,
           shadow, status
  checks/  battery
  stack/   compose_gen, bringup, golden, sender
  chain/   writes
  core/    proc, rpc, nodes, topology, prng, setops, chainpaced, events, termio, policy

Invocation: `python -m dpos_harness <group> <command>` — see cli.py.

This is a TRANSLATION port of the bash sim harness: verdict semantics, fail ids, belts,
thresholds and every knob's DEFAULT are preserved exactly; the bash idioms (subshell
write-loss, set-e branch-tail, assoc-array mis-parse) die structurally. Only the knob
PREFIXES moved with the port — the bash `SOAK_*` are `SIM_*` here, the blaster's `LH_*`
are `LOAD_*` — because the thing is a simulation and the blaster is a framework tool.
"""
