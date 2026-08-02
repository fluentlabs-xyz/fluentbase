"""smoke-cert-catchup (standalone) — `scripts/case-cert-catchup.sh`.

The deterministic, fast gate for the executor cert-budget catch-up fix (task
`dpos-cert-budget-catchup-recover-stall`).

TUNED GENESIS, and the tuning IS the assertion. `EPOCH_BLOCK_INTERVAL=64` mirrored into
`EPOCH_INTERVAL` — the container's genesis-init env and the host-side chain math must agree, and
bash mirrors them for exactly that reason. At the default 32 the pure-park window
(`gap < re_jump_threshold = min(1024, epochBlockInterval)`) is too shallow to force a multi-block
derive-walk at all, so the case would pass without ever exercising the path it exists to gate.

64 IS ALSO THE CEILING: it bounds how far the victim may fall behind before `maybe_re_jump`
fast-forwards past the walk. Raising it to 128 to buy headroom was tried live on 2026-07-31 and
the stack does not boot at that interval — `vo.CATCHUP_EPOCH_INTERVAL` carries the evidence. The
gap is the lever instead, and its bound is `vo.catchup_gap_ceiling`.

`${EPOCH_BLOCK_INTERVAL:-64}` keeps bash's default-from-env spelling: an operator who exports a
different interval gets it, and `EPOCH_INTERVAL` follows it unconditionally so the two can never
drift.

A PER-CASE COMPOSE OVERLAY too, and it grants exactly one thing: `NET_ADMIN`, which `tc qdisc
add` needs. The case adds `tc netem delay` to the victim for its catch-up window — without it the
guard-#2 park fires at no gap on a zero-latency LAN and the anti-vacuity gate is unsatisfiable
(see `vo.CATCHUP_NETEM_DELAY_MS`). It rides `driver.run(overlays=…)` rather than
`DPOS_EXTRA_COMPOSE` — same file bash names in its export, kept in the case rather than in the
environment it happened to be launched with — and deliberately NOT an edit to the shared
`docker-compose.yml`, which every other case also brings up.

Standalone and heavy (~8-10 min: ~5-6 min to the DKG-safe point at interval=64, then the cycle).
"""

from __future__ import annotations

import os

from . import asserts_onchain, driver, verdicts_onchain as vo


def run_case(argv=None) -> int:
    interval = os.environ.get("EPOCH_BLOCK_INTERVAL", str(vo.CATCHUP_EPOCH_INTERVAL))
    return driver.run("smoke-cert-catchup", [asserts_onchain.assert_cert_catchup], argv,
                      overlays=(vo.CATCHUP_OVERLAY,),
                      exports={"EPOCH_BLOCK_INTERVAL": interval, "EPOCH_INTERVAL": interval})
