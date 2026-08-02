"""test_metric_fixtures.py — parser parity against REAL PRODUCTION BYTES.

The metric/log parsers were tuned against months of real bundles; a Python parser that scrapes
the same family but rounds differently or greps a subtly different regex produces a DIFFERENT
verdict on the same chain (analysis §3.4 — the worst outcome for a safety net). These fixtures
are VERBATIM lines from a real sim forensics bundle
(soak-out/bundle-20260720T170507Z/rpc/metrics-19100.txt and metrics-19200.txt), so the parsers
are proven against production bytes, not synthetic stubs.

The prometheus double-`_total` suffix (`beacon_seed_active_total_total 5963`) is exactly the
byte shape the bash `awk index($0,f)` substring parse handles — a naive anchored-suffix match
would MISS it, which is why the beacon seam uses substring, not gauge_val."""

from dpos_harness.core import nodes


# ── VERBATIM production bytes (bundle-20260720T170507Z/rpc/metrics-19100.txt, commonware :9100) ──
COMMONWARE_METRICS = """\
# HELP beacon_seed_active_total the beacon seed active counter
# TYPE beacon_seed_active_total counter
beacon_seed_active_total_total 5963
# HELP dkg_ceremony_fail_total dkg ceremony failures
dkg_ceremony_fail_total_total 0
# HELP dkg_ceremony_ok_total dkg ceremony successes
dkg_ceremony_ok_total_total 8
epoch_engine_demoted_key_divergence_total_total 0
epoch_engine_demoted_no_polynomial_total_total 0
epoch_engine_demoted_rotated_out_total_total 0
"""

# ── VERBATIM production bytes (bundle-20260720T170507Z/rpc/metrics-19200.txt, reth :9200) ──
RETH_METRICS = """\
reth_dpos_executor_eager_finalized_derive_total{outcome="hit"} 5963
reth_dpos_executor_eager_finalized_derive_total{outcome="miss"} 238
reth_dpos_executor_stale_finalization_pruned_total 1
reth_dpos_parent_seed_embedded_total 1972
reth_sync_block_validation_deferred_trie_compute_duration_sum 0
reth_sync_block_validation_deferred_trie_compute_duration_count 0
"""


def test_beacon_seed_active_real_bytes():
    """sim_beacon_seed_delta — the warm-debt-warm signal. `beacon_seed_active_total_total 5963`."""
    assert nodes.beacon_metric_value(COMMONWARE_METRICS, "beacon_seed_active") == 5963


def test_dkg_ceremony_ok_real_bytes():
    """sim_beacon_share_delta — the qualified-signer / dkg-member-ready signal. Real value: 8."""
    assert nodes.beacon_metric_value(COMMONWARE_METRICS, "dkg_ceremony_ok") == 8


def test_dkg_ceremony_fail_and_demote_zero():
    """The promote/refill-shareless watchdog POSITIVE signals — real bytes read 0 (healthy)."""
    assert nodes.beacon_metric_value(COMMONWARE_METRICS, "dkg_ceremony_fail") == 0
    assert nodes.beacon_metric_value(COMMONWARE_METRICS,
                                     "epoch_engine_demoted_no_polynomial") == 0


def test_absent_family_is_minus_one():
    """An absent family reads -1 (a down/booting container never false-confirms), not 0."""
    assert nodes.beacon_metric_value(COMMONWARE_METRICS, "no_such_metric_family") == -1


def test_metric_val_substring_not_anchored():
    """The load-bearing property: the beacon parse is SUBSTRING (index($0,f)), so the double-
    `_total` suffix is handled. A comment line for the same family is skipped."""
    assert nodes.metric_val(COMMONWARE_METRICS, "beacon_seed_active", "") == "5963"


def test_summary_agg_real_reth_pair():
    """summary_agg over a real reth `_sum`/`_count` pair (both 0 in this capture)."""
    s, c = nodes.summary_agg(RETH_METRICS,
                             "reth_sync_block_validation_deferred_trie_compute_duration")
    assert (s, c) == (0.0, 0)


def test_labelled_gauge_last_value():
    """metric_val takes the LAST matching line's last field — a labelled family's final label set.
    `reth_dpos_executor_eager_finalized_derive_total{outcome="miss"} 238` is last → 238."""
    assert nodes.metric_val(RETH_METRICS, "eager_finalized_derive", "") == "238"
