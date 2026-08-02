"""test_prod_substrate.py — the production-path SUBSTRATE (`lib.sh:589-1362`), trap by trap.

Chunk 5a ports the ~620 lines of `lib.sh` that all five production-path cases stand on. Nearly
every defensive oddity in that range is a diagnosed production failure with a bundle id in its
comment, so the suite here is organised by TRAP rather than by function: each section names the
behaviour bash encodes, drives it through EVERY outcome (three cases for a three-valued return,
not two), and asserts the failure direction as hard as the success one.

The FAIL direction is the point. A live run only ever walks the pass side of these — a chain that
does not blip never produces an unreadable `getDkgQual`, and a funded owner-0 never hits the mint
floor — so if a code path that distinguishes two failures is only exercised by a green run, it is
not exercised at all.
"""

from __future__ import annotations

import io
import os
import sys

import pytest

from dpos_harness.cases.smoke import prod, verdicts_prod as V
from dpos_harness.chain.writes import Chain, ChainError
from dpos_harness.core.chainpaced import ChainPaced
from dpos_harness.core.proc import RunResult, Runner
from dpos_harness.core.spammer import SpammerPool
from dpos_harness.stack import production_path as PP


# ── helpers ────────────────────────────────────────────────────────────────────────────

class FakeRunner(Runner):
    """A Runner whose exec is a scripted answer table. Records every argv like the real one, so
    the transcript oracle is unchanged; answers by NOTE, which is what the call sites carry."""

    def __init__(self, answers=None, **kw):
        super().__init__(**kw)
        self.answers = dict(answers or {})
        self.calls = []

    def _reply(self, note, argv):
        a = self.answers.get(note)
        if callable(a):
            a = a(argv)
        if a is None:
            return RunResult(argv=argv, stdout="", rc=0)
        if isinstance(a, RunResult):
            return RunResult(argv=argv, stdout=a.stdout, stderr=a.stderr, rc=a.rc,
                             timed_out=a.timed_out)
        return RunResult(argv=argv, stdout=str(a), rc=0)

    def run(self, argv, env_overlay=None, timeout=90, note="", cwd=None) -> str:
        argv = [str(x) for x in argv]
        self.calls.append((note, argv))
        self.log.append(_inv(argv, note, env_overlay, cwd))
        return self._reply(note, argv).stdout.strip()

    def run_capture(self, argv, env_overlay=None, timeout=90, note="", cwd=None,
                    binary=False) -> RunResult:
        argv = [str(x) for x in argv]
        self.calls.append((note, argv))
        self.log.append(_inv(argv, note, env_overlay, cwd))
        return self._reply(note, argv)

    def run_ok(self, argv, env_overlay=None, timeout=90, note="") -> bool:
        return self.run_capture(argv, env_overlay=env_overlay, timeout=timeout, note=note).ok

    def run_checked(self, argv, env_overlay=None, timeout=600, note="", cwd=None) -> RunResult:
        r = self.run_capture(argv, env_overlay=env_overlay, timeout=timeout, note=note, cwd=cwd)
        if not r.ok:
            from dpos_harness.core.proc import ProcError
            raise ProcError(r, note)
        return r

    def notes(self):
        return [n for n, _ in self.calls]

    def argv_for(self, note):
        return [a for n, a in self.calls if n == note]


def _inv(argv, note, env_overlay, cwd):
    from dpos_harness.core.proc import Invocation
    return Invocation(argv=argv, env=dict(env_overlay or {}), kind="run", note=note, cwd=cwd)


def _chain(answers=None, dry=False, **facts):
    r = FakeRunner(answers, dry=dry)
    facts.setdefault("RPC", "http://localhost:8545")
    facts.setdefault("STAKING_RT", "0xstaking")
    facts.setdefault("CHAIN_CONFIG_RT", "0xcfg")
    facts.setdefault("GOV_ADDR", "0xgov")
    return Chain(runner=r, **facts), r


def _advancing(start=0, step=1):
    """A block cursor that MOVES. Every chain-paced budget is denominated in observed chain
    progress, so a constant cursor never spends the budget and the wait is unbounded — which is
    the primitive behaving correctly (a frozen chain is the finalize-stall invariants' problem,
    not a wait's) and would hang the test."""
    box = [start - step]

    def nxt():
        box[0] += step
        return box[0]
    return nxt


def _states(*seq):
    """A `state(pid)` answer table that WALKS: `pp_gov_action` polls it twice for two different
    questions — Active(1) at the start, then Succeeded(4) — and a constant answer can only satisfy
    one of them."""
    it = iter(seq)
    last = [seq[-1]]

    def nxt(_argv):
        try:
            last[0] = next(it)
        except StopIteration:
            pass
        return last[0]
    return nxt


def _paced(cp_source=None, **kw):
    """A ChainPaced wired for a unit test: advancing cursor, no sleeps, no stderr noise."""
    kw.setdefault("block_number", cp_source or _advancing())
    kw.setdefault("head_age_s", lambda: 0)
    kw.setdefault("sleep", lambda _s: None)
    kw.setdefault("warn", lambda _n: None)
    return ChainPaced(**kw)


@pytest.fixture(autouse=True)
def _no_sleep(monkeypatch):
    """Every retry ladder in this substrate sleeps 1-3 s between attempts. The ladders are what is
    under test; the waiting is not."""
    monkeypatch.setattr("time.sleep", lambda *_a, **_k: None)


@pytest.fixture(autouse=True)
def _clean_gov_env(monkeypatch):
    """The three voter-selection modes are read out of the environment, so a leaked value from an
    earlier test would silently change which mode a later one exercises."""
    for k in ("PP_GOV_VOTERS", "PP_GOV_VOTE_ALL", "PP_GOV_VOTER_IDX", "PP_COMMITTEE_SIZE",
              "PP_PEERS"):
        monkeypatch.delenv(k, raising=False)


# ══ TRAP 1 — bash dynamic scope ════════════════════════════════════════════════════════
#
# `_pp_gov_active` writes the caller's `state`, `_pp_gov_exec_confirmed` writes the caller's
# `ex_now`, `_pp_wait_epochs_reached` reads the caller's `_cp_wep_start`/`_cp_wep_n`. There is no
# Python equivalent; done naively the failure messages go blank and the epoch wait never ends.

def test_gov_not_active_failure_names_the_last_observed_state():
    """bash: `echo "FAIL pp_gov_action: proposal not Active (state=$state)"`. The state is the
    whole diagnosis — 0=Pending means votingDelay is not 0, 3=Defeated means it never opened —
    and a message that omits it says only that something did not happen."""
    chain, r = _chain({"gov-state": "3", "gov-hash": "42"})
    chain.cp = _paced()
    with pytest.raises(ChainError) as e:
        chain.gov_action("0xtarget", "0xdata", "some-action")
    assert "state=3" in e.value.message
    assert "some-action" in e.value.message


def test_gov_not_active_message_tracks_a_different_state():
    """The other direction: a different last-observed state must produce a DIFFERENT message. A
    hard-coded string would satisfy the test above and nothing else."""
    chain, _ = _chain({"gov-state": "0", "gov-hash": "42"})
    chain.cp = _paced()
    with pytest.raises(ChainError) as e:
        chain.gov_action("0xtarget", "0xdata", "some-action")
    assert "state=0" in e.value.message and "state=3" not in e.value.message


def test_gov_execute_no_receipt_failure_names_the_last_read_nonce():
    """bash: `sender nonce stuck at $ex_pre, last read $ex_now`. `ex_now` is the cond_fn's write
    into the caller's frame, and it is what distinguishes "the tx never mined" from "the nonce
    read is failing"."""
    chain, _ = _chain({"gov-state": _states("1", "4"), "gov-hash": "42", "gov-deadline": "10",
                       "gov-execute": RunResult(argv=[], stdout="", rc=1, stderr="boom"),
                       "nonce": "7", "runtime-cat": "aa", "owner-addr": "0xowner"})
    chain.cp = _paced()
    with pytest.raises(ChainError) as e:
        chain.gov_action("0xtarget", "0xdata", "some-action")
    assert "nonce stuck at 7" in e.value.message and "last read 7" in e.value.message


def test_wait_epochs_captures_the_start_epoch_once():
    """`pp_wait_epochs` latches `_cp_wep_start` BEFORE the loop and the predicate reads it out of
    that frame. A port that re-read the start each poll would compare `epoch >= epoch + n`, which
    is false forever — the wait would never terminate on a perfectly healthy chain."""
    epochs = iter([5, 5, 6, 7, 8])
    ctx = _prod_ctx()
    ctx.current_epoch = lambda **_k: next(epochs)
    ctx.cp = _paced(block_number=None, current_epoch=lambda: 5)
    assert ctx.wait_epochs(2) is True


def test_wait_epochs_fails_on_a_budget_not_on_a_moving_target():
    """The other direction: an epoch cursor that does NOT advance must terminate on the epochs
    budget rather than spin. The budget is `n` epochs in the `epochs` domain — the wait follows
    the chain and only gives up once the chain has made the agreed progress without the
    condition."""
    ctx = _prod_ctx()
    ctx.current_epoch = lambda **_k: 5
    cursor = iter([5, 6, 7, 8, 9, 10])
    ctx.cp = _paced(block_number=None, current_epoch=lambda: next(cursor))
    assert ctx.wait_epochs(2) is False


# ══ TRAP 2 — chain_paced_step's FIVE verdicts, and read_fail is not a failure ═══════════

@pytest.mark.parametrize("verdict,kw", [
    ("met", dict(met=1, cursor=0, budget=5, age=0)),
    ("waiting", dict(met=0, cursor=1, budget=5, age=0)),
    ("budget", dict(met=0, cursor=5, budget=5, age=0)),
    ("frozen", dict(met=0, cursor=1, budget=5, age=999)),
    ("read_fail", dict(met=0, cursor=None, budget=5, age=0)),
])
def test_chain_paced_step_one_test_per_verdict(verdict, kw):
    """One test per value. Collapsing the five onto a boolean loses the attribution `_cp_attribute`
    writes, which is the only thing that says WHY a wait ended."""
    cp = ChainPaced(block_number=lambda: kw["cursor"], head_age_s=lambda: kw["age"],
                    warn=lambda _n: None)
    cp.cp_start["k"] = 0
    assert cp.step("k", kw["met"], "blocks", kw["budget"]) == verdict


def test_read_fail_keeps_polling_and_never_terminates_the_wait():
    """§2.4 item 7 — `wait_chain_paced` KEEPS POLLING on read_fail; only budget and frozen end it.
    A transient RPC blip must not fail a blocking wait, and a port that treated the five verdicts
    as ok/not-ok would fail the run on one bad read."""
    reads = iter([None, None, 1, 2])
    cp = ChainPaced(block_number=lambda: next(reads), head_age_s=lambda: 0,
                    sleep=lambda _s: None, warn=lambda _n: None)
    hits = iter([False, False, False, True])
    assert cp.wait("k", lambda: next(hits), "blocks", 10) is True


def test_a_persistent_read_fail_still_ends_on_the_budget_once_reads_recover():
    """The other direction: read_fail being tolerated must not make the wait unconditionally
    patient. Once the cursor becomes readable again the same budget guard applies."""
    reads = iter([None, 0, 10, 10])
    cp = ChainPaced(block_number=lambda: next(reads), head_age_s=lambda: 0,
                    sleep=lambda _s: None, warn=lambda _n: None)
    assert cp.wait("k", lambda: False, "blocks", 5) is False


def test_attribution_reaches_stderr_when_there_is_no_event_sink():
    """`_cp_attribute` (lib.sh:869) emits through `soak_event` when the sim's bundle writer is
    sourced and FALLS BACK to `echo "WARN (chain-paced): …" >&2` otherwise — "so a budget/frozen
    verdict [in a lib-only case] is still surfaced". The port kept only the list, which nothing
    reads, so every attribution the machine ever produced went nowhere."""
    buf = io.StringIO()
    old, sys.stderr = sys.stderr, buf
    try:
        cp = ChainPaced(block_number=lambda: 10, head_age_s=lambda: 0, sleep=lambda _s: None)
        assert cp.wait("k", lambda: False, "blocks", 0) is False
    finally:
        sys.stderr = old
    assert "WARN (chain-paced)" in buf.getvalue()
    assert "chain-paced 'k'" in cp.fail_msg


# ══ TRAP 3 — pp_fund_eth's three distinct codes ════════════════════════════════════════

def test_fund_eth_balance_read_failure_is_code_3_not_a_floor():
    """bundle-20260716T203448Z case 3: a FAILED `cast balance` read must not collapse to bal=0,
    which would be indistinguishable from a floor hit."""
    chain, _ = _chain({"balance": "not-a-number", "owner-addr": "0xowner"})
    assert chain.fund_eth("0xto", 1) == V.FUND_READ
    assert chain.fund_eth_bal == "" and chain.fund_eth_err == ""


def test_fund_eth_genuine_floor_is_code_1_and_surfaces_the_balance():
    chain, _ = _chain({"balance": "1", "owner-addr": "0xowner"})
    assert chain.fund_eth("0xto", 1) == V.FUND_FLOOR
    assert chain.fund_eth_bal == 1


def test_fund_eth_send_failure_is_code_2_and_surfaces_the_error_line():
    """bundle-20260716T203448Z case 2: a stalled/congested chain is a SEND failure, and the budget
    is fine. The error line is what tells the operator which."""
    chain, _ = _chain({"balance": str(10 ** 18), "owner-addr": "0xowner", "gas-price": "7",
                       "fund-eth": RunResult(argv=[], stdout="", rc=1,
                                             stderr="error: nonce too low")})
    assert chain.fund_eth("0xto", 1) == V.FUND_SEND
    assert "nonce too low" in chain.fund_eth_err


def test_fund_eth_success_is_code_0():
    chain, _ = _chain({"balance": str(10 ** 18), "owner-addr": "0xowner", "gas-price": "7",
                       "fund-eth": '{"status":"0x1"}'})
    assert chain.fund_eth("0xto", 1) == V.FUND_OK


def test_only_the_floor_code_means_budget_exhausted():
    """The predicate that closes the bug: `code != 0` reported all three as budget exhaustion."""
    assert V.fund_is_budget_exhausted(V.FUND_FLOOR)
    for code in (V.FUND_OK, V.FUND_SEND, V.FUND_READ):
        assert not V.fund_is_budget_exhausted(code)


def test_every_fund_code_has_its_own_sentence():
    """Four codes, four distinct attributions — and an unknown code is NAMED rather than defaulted
    into one of them."""
    reasons = {V.fund_reason(c) for c in (0, 1, 2, 3)}
    assert len(reasons) == 4
    assert "unknown" in V.fund_reason(9)


def test_fund_eth_retries_the_balance_read_once_before_giving_up():
    """bash reads the balance twice (`for attempt in 1 2`) before classifying a read failure. One
    attempt would make a single blip a code-3."""
    seq = iter(["", str(10 ** 18)])
    chain, r = _chain({"balance": lambda _a: next(seq), "owner-addr": "0xowner",
                       "gas-price": "7", "fund-eth": '{"status":"0x1"}'})
    assert chain.fund_eth("0xto", 1) == V.FUND_OK
    assert len(r.argv_for("balance")) == 2


def test_fund_eth_is_a_noop_under_dry():
    chain, r = _chain(dry=True)
    chain.p.dry = True
    assert chain.fund_eth("0xto", 1) == V.FUND_OK
    assert "fund-eth" not in r.notes()


# ══ TRAP 4 — pp_dkg_qual's "" is UNREADABLE, not deferred ══════════════════════════════

@pytest.mark.parametrize("raw,token,state", [
    ("  true", "1", V.DKG_QUALIFIED),
    (" false", "0", V.DKG_DEFERRED),
    ("", "", V.DKG_UNREADABLE),
    ("Error: server returned an error", "", V.DKG_UNREADABLE),
    ("0x", "", V.DKG_UNREADABLE),
])
def test_dkg_qual_is_three_valued(raw, token, state):
    """A test per value. Mapping `""` onto "deferred" turns an RPC blip into a real verdict about
    the chain — that the incoming committee's DKG did not qualify."""
    assert V.dkg_qual_token(raw) == token
    assert V.dkg_qual_state(token) == state


def test_unreadable_is_not_qualified_either():
    """`dkg_qual_is_set` is `token == "1"`, not `state != DEFERRED`. An unreadable read is not a
    qualification."""
    assert V.dkg_qual_is_set("1")
    assert not V.dkg_qual_is_set("0")
    assert not V.dkg_qual_is_set("")


def test_the_two_dkg_qual_state_implementations_agree():
    """`checks/battery.py` has the same three-valued classifier for the sim. The case layer must
    not import a 2000-line detector module for five lines, so the two are separate — and this is
    what keeps them from drifting, rather than either mirroring the other."""
    from dpos_harness.checks import battery
    for token in ("1", "0", "", "true", None, "-1"):
        assert V.dkg_qual_state(token) == battery.dkg_qual_state(token)
        assert V.dkg_qual_is_set(token) == battery.dkg_qual_is_set(token)


# ══ TRAP 5 — the `[sci]` suffix strip ══════════════════════════════════════════════════

def test_cfg_read_strips_the_pretty_printed_suffix():
    """`cast call …(uint64)` prints `22920 [2.292e4]`. `printf '%d'` on the whole string emits the
    int AND exits non-zero, so bash's `|| echo 0` APPENDS a `0` → `229200`: an activation block
    past the head, which pins the epoch to 0 and disables all churn without an error."""
    chain, _ = _chain({"cfg-read": "22920 [2.292e4]"})
    got = chain._pp_cfg_read_retry("getDposActivationBlock()(uint64)")
    assert got == 22920
    assert got != 229200


def test_first_token_leaves_a_small_uint_alone():
    """The other direction: a value below cast's pretty-print threshold arrives without a suffix
    and must come through unchanged."""
    assert V.first_token("64") == "64"
    assert V.cfg_value(True, "64") == 64


def test_a_successful_but_empty_cfg_read_is_zero_not_a_retry():
    """bash branches on cast's EXIT CODE. A getter that answered and printed nothing yields 0 via
    `printf '%d' "" || echo 0` — it is NOT a transient. Retrying it and then reporting a transient
    failure makes the caller fall back to a MEMOIZED interval, i.e. compute a non-zero epoch off a
    ChainConfig that just said there is none."""
    assert V.cfg_value(True, "") == 0
    chain, r = _chain({"cfg-read": RunResult(argv=[], stdout="", rc=0)})
    assert chain._pp_cfg_read_retry("getEpochBlockInterval()(uint32)") == 0
    assert len(r.argv_for("cfg-read")) == 1          # answered on the first attempt


def test_a_failing_cfg_read_retries_three_times_then_reports_transient():
    assert V.cfg_value(False, "anything") is None
    chain, r = _chain({"cfg-read": RunResult(argv=[], stdout="", rc=1, stderr="no such host")})
    assert chain._pp_cfg_read_retry("getEpochBlockInterval()(uint32)") is None
    assert len(r.argv_for("cfg-read")) == 3


# ══ TRAP 6 — pp_committee's `|| true` ══════════════════════════════════════════════════

def test_an_empty_committee_is_an_empty_string_not_an_abort():
    """§2.4 item 8: without the trailing `|| true` an empty committee makes `grep .` exit 1, and
    under the caller's `pipefail` that ABORTS the whole run instead of yielding the "" the
    assertion can report. A Python port that raised would invert the same intent."""
    assert V.committee_set("") == ""
    assert V.committee_set("[]") == ""
    assert V.committee_set("Error: execution reverted") == ""


def test_a_committee_is_sorted_and_lowercased():
    """Set membership must compare regardless of on-chain ordering and casing, or a case comparing
    two epochs' committees would see a rotation in every re-ordering."""
    raw = "[0xBB" + "b" * 38 + ", 0xAA" + "a" * 38 + "]"
    got = V.committee_set(raw)
    assert got == " ".join(sorted(got.split()))
    assert got == got.lower()
    assert len(got.split()) == 2


def test_ctx_committee_never_raises_on_an_unreadable_read(monkeypatch):
    ctx = _prod_ctx()
    monkeypatch.setattr("dpos_harness.core.nodes.staking_call", lambda *_a, **_k: "")
    assert ctx.committee(3) == ""
    assert ctx.committee_has("0xabc", 3) is False


# ══ TRAP 7 — pp_current_epoch memoizes deliberately ════════════════════════════════════

def test_current_epoch_reuses_the_last_good_geometry_across_a_blip():
    """§2.4 item 15. The interval and activation block are on-chain CONSTANTS after bring-up, so a
    transient double-read failure must NOT default them to 64/0 — both INFLATE the epoch, and an
    inflated epoch false-trips the growth-landing watchdog. A stateless port re-creates that."""
    seq = iter([RunResult(argv=[], stdout="32", rc=0), RunResult(argv=[], stdout="128", rc=0)]
               + [RunResult(argv=[], stdout="", rc=1)] * 8)
    chain, _ = _chain({"cfg-read": lambda _a: next(seq)})
    chain._head_hex = lambda: hex(1000)
    first = chain.current_epoch()
    second = chain.current_epoch()                 # both reads now FAIL — the memo answers
    assert first == second == (1000 - 128) // 32


def test_a_stateless_read_would_have_inflated_the_epoch():
    """The counterfactual, made explicit: with the bash defaults (interval 64, act 0) the same
    head yields a DIFFERENT, larger epoch. That is the false watchdog trip the memo prevents."""
    memoized = V.current_epoch(head=1000, act=128, interval=32)
    stateless = V.current_epoch(head=1000, act=0, interval=64)
    assert stateless != memoized


@pytest.mark.parametrize("head,act,interval,want", [
    (1000, 128, 32, 27),
    (100, 128, 32, 0),        # before activation — the relative epoch is 0, never negative
    (1000, 128, 0, 0),        # no interval yet — fail-SAFE, not a ZeroDivisionError
])
def test_current_epoch_arithmetic(head, act, interval, want):
    assert V.current_epoch(head, act, interval) == want


# ══ TRAP 8 — pp_ensure_blend probes before it sends ════════════════════════════════════

def test_ensure_blend_skips_the_transfer_when_the_balance_is_already_non_zero():
    """§2.4 item 10: a transfer is NOT idempotent and a timed-out receipt does not mean it failed,
    so the balance is checked FIRST. A fresh idx starts at 0, so any non-zero balance means the
    transfer already landed."""
    chain, r = _chain({"balanceOf": "5"})
    assert chain.ensure_blend("0xtoken", "0xto", 1) is True
    assert "token-transfer" not in r.notes()


def test_ensure_blend_sends_while_the_balance_is_still_zero_and_is_bounded():
    """The other direction, and the bound: four attempts, then a final probe. Unbounded retrying
    against a chain that will never accept the transfer is how a night-long run stalls."""
    chain, r = _chain({"balanceOf": "0"})
    assert chain.ensure_blend("0xtoken", "0xto", 1) is False
    assert len(r.argv_for("token-transfer")) == 4


def test_ensure_blend_notices_a_transfer_that_landed_late():
    """The whole reason for the probe: a send whose receipt timed out still lands, and the NEXT
    balance check is what sees it."""
    seq = iter(["0", "9"])
    chain, r = _chain({"balanceOf": lambda _a: next(seq)})
    assert chain.ensure_blend("0xtoken", "0xto", 1) is True
    assert len(r.argv_for("token-transfer")) == 1


# ══ TRAP 9 — the votes are --async, and there are three voter modes ════════════════════

def test_votes_go_out_async():
    """§2.4 item 11: five receipt-awaited sends cost 1-2 blocks EACH and can overrun the 10-block
    voting window. The Succeeded poll is the real synchronization point."""
    chain, r = _chain({"gov-state": "1", "gov-hash": "42", "gov-deadline": "10",
                       "gov-execute": '{"status":"0x1"}', "owner-addr": "0xowner"})
    chain.cp = _paced()
    chain._gov_wait_succeeded = lambda *_a: None
    chain.gov_action("0xtarget", "0xdata", "d")
    votes = r.argv_for("gov-vote")
    assert votes and all("--async" in v for v in votes)


@pytest.mark.parametrize("voters,vote_all,explicit,want", [
    (5, False, None, [0, 1, 2, 3, 4]),        # ceil(2/3*5)+1 = 5 -> clamped to voters
    (9, False, None, [0, 1, 2, 3, 4, 5, 6]),  # ceil(2/3*9)+1 = 7
    (9, True, None, list(range(9))),          # PP_GOV_VOTE_ALL
    (9, False, "3 7 11", [3, 7, 11]),         # PP_GOV_VOTER_IDX wins over both
    (9, True, [2], [2]),                      # ...even over vote_all
    (0, False, None, [0]),                    # clamped to >= 1
])
def test_the_three_voter_modes(voters, vote_all, explicit, want):
    assert V.voter_list(voters, vote_all=vote_all, explicit=explicit) == want


def test_vote_all_is_read_from_the_environment(monkeypatch):
    """`PP_GOV_VOTE_ALL` had no port at all: the sim could pass an explicit list, but the env
    switch five bash callers can set was simply absent."""
    chain, _ = _chain()
    monkeypatch.setenv("PP_GOV_VOTERS", "9")
    assert chain._voter_list(None) == list(range(7))
    monkeypatch.setenv("PP_GOV_VOTE_ALL", "1")
    assert chain._voter_list(None) == list(range(9))


def test_voter_idx_is_read_from_the_environment(monkeypatch):
    chain, _ = _chain()
    monkeypatch.setenv("PP_GOV_VOTER_IDX", "4 8 15")
    assert chain._voter_list(None) == [4, 8, 15]


def test_a_reverted_execute_is_not_confirmed_by_the_advancing_nonce():
    """A REVERTED tx consumes its nonce exactly as a successful one does. Collapsing "receipt says
    reverted" into "no receipt" therefore made the nonce confirm PASS, and a governance action
    that reverted on-chain was reported as applied."""
    chain, r = _chain({"gov-state": _states("1", "4"), "gov-hash": "42", "gov-deadline": "10",
                       "owner-addr": "0xowner", "nonce": "7",
                       "gov-execute": '{"status":"0x0"}',
                       "gov-execute-reason": "Error: NotAuthorized()"})
    chain.cp = _paced()
    with pytest.raises(ChainError) as e:
        chain.gov_action("0xtarget", "0xdata", "d")
    assert e.value.reason_id == "gov-execute-reverted"
    assert "NotAuthorized" in e.value.message
    assert r.argv_for("gov-execute-reason"), "the revert reason was never asked for"


# ══ TRAP 10 — awk '{print $1}' on a big-uint cast return ═══════════════════════════════

def test_the_proposal_id_is_stripped_of_its_sci_suffix():
    """`hashProposal` returns a uint256, which cast prints as `9857… [9.86e76]`. The ` [9.86e76]`
    must be stripped or it fails to re-parse as a uint256 argument to state()/castVote()/execute()
    — a silent parser error, not an exception."""
    pid = "98570000000000000000000000000000000000000000000000000000000000000000000000000"
    chain, r = _chain({"gov-hash": f"{pid} [9.86e76]", "gov-state": "1", "gov-deadline": "10",
                       "gov-execute": '{"status":"0x1"}', "owner-addr": "0xowner"})
    chain.cp = _paced()
    chain._gov_wait_succeeded = lambda *_a: None
    chain.gov_action("0xtarget", "0xdata", "d")
    assert r.argv_for("gov-vote")[0][4] == pid


def test_the_status_byte_survives_a_pretty_printed_stake_field():
    """`getValidatorStatus` returns (address, uint8, uint256, …) and the uint256 stake prints as
    `1000000000000000000 [1e18]`. A reader that scanned for "a small number" would find the
    exponent; a field-position parse does not."""
    out = "0xabc\n1\n1000000000000000000 [1e18]\n0\n0\n0\n0\n0"
    assert V.status_byte(out) == "1"
    assert V.status_byte("") == ""


# ══ TRAP 11 — `exit 1` fired the EXIT trap; a raise does not ═══════════════════════════

def test_a_failed_bring_up_still_runs_all_three_cleanup_steps(monkeypatch, tmp_path):
    """§2.4 item 12. bash's `exit 1` inside `pp_bring_up_rotation` TERMINATED the process, which
    fired `trap cleanup EXIT` — and cleanup is three things: `pp_spammer_stop; rm -f "$MANIFEST";
    tear_down`. A Python `raise` only unwinds, so a missing `finally` leaks a spammer thread, a
    stale manifest AND a running six-node devnet on every failed bring-up."""
    manifest = tmp_path / "runtime-deployment.json"
    manifest.write_text("{}")
    stopped = []

    def boom(self):
        raise PP.RotationBringUpError("smoke-x", "bare chain did not converge")

    monkeypatch.setattr(PP.RotationBringUp, "run", boom)
    monkeypatch.setattr(SpammerPool, "stop", lambda self: stopped.append(True))
    seen = {}
    monkeypatch.setattr(prod, "tear_down", lambda r: seen.setdefault("down", True))

    rc = prod.run("smoke-x", [], manifest=str(manifest))
    assert rc == 1
    assert stopped, "the spammer was never reaped"
    assert not manifest.exists(), "the deploy manifest was left for the next run to read"
    assert seen.get("down"), "the stack was never torn down"


def test_a_cleanup_step_that_raises_does_not_skip_the_others(monkeypatch, tmp_path):
    """bash's three cleanup commands each swallow their own failure. In Python a raise in the
    first would skip the other two — which is how a leaked spammer outlives the stack it was
    pressuring."""
    manifest = tmp_path / "m.json"
    manifest.write_text("{}")

    def explode(self):
        raise RuntimeError("thread join blew up")

    monkeypatch.setattr(PP.RotationBringUp, "run", lambda self: self)
    monkeypatch.setattr(SpammerPool, "stop", explode)
    seen = {}
    monkeypatch.setattr(prod, "tear_down", lambda r: seen.setdefault("down", True))
    assert prod.run("smoke-x", [], manifest=str(manifest)) == 0
    assert not manifest.exists() and seen.get("down")


def test_a_passing_case_also_cleans_up(monkeypatch, tmp_path):
    manifest = tmp_path / "m.json"
    manifest.write_text("{}")
    monkeypatch.setattr(PP.RotationBringUp, "run", lambda self: self)
    seen = {}
    monkeypatch.setattr(prod, "tear_down", lambda r: seen.setdefault("down", True))
    assert prod.run("smoke-x", [lambda ctx: None], manifest=str(manifest)) == 0
    assert seen.get("down")


# ══ TRAP 12 — the timeout wrappers are the hang guards ═════════════════════════════════

def test_runtime_cat_is_bounded():
    """`timeout 15 docker compose exec …` (lib.sh:611) — FLK-2. This runs per-tick via
    `committee_has`, and a wedged docker daemon must not hang the whole tick. The `timeout(1)`
    binary becomes subprocess's own `timeout=`, so the RECORDED argv stays the bare docker line."""
    seen = {}

    class Cap(Runner):
        def _exec(self, argv, overlay, timeout, cwd, text=True, stdin=None):
            seen["timeout"] = timeout
            raise RuntimeError("not executed — the ceiling is what is under test")

    c = Chain(runner=Cap(), STAKING_RT="0x0")
    assert c.runtime_cat("keys/owner-0.hex") == ""      # `run` degrades to "" on any failure
    assert seen["timeout"] == 15
    assert c.p.log[-1].argv[0] == "docker", "the recorded argv must be the bare docker line"


def test_the_spammer_send_is_bounded():
    """`timeout 90 cast send …` (lib.sh:1220) — a transient follower wedge must make the loop
    RETRY, not hang. A `subprocess.run` without `timeout=` reintroduces the wedge."""
    from dpos_harness.core import spammer
    assert spammer.SEND_TIMEOUT_S == 90
    seen = {}
    pool = SpammerPool(dry=False)

    def one_shot(argv, **kw):
        seen["timeout"] = kw.get("timeout")
        seen["argv"] = argv
        pool._stop.set()                       # one pass, then the loop exits
        return True

    pool.p.run_ok = one_shot
    pool._loop("0xkey", "0xto", "http://rpc", "n")
    assert seen["timeout"] == 90
    assert seen["argv"][:2] == ["cast", "send"]


# ══ the spammer POOL — stop() reaps every spammer, not the last ════════════════════════

def test_stop_reaps_every_spammer_ever_started():
    """`PP_SPAMMER_PIDS` is an ARRAY; `PP_SPAMMER_PID` is only the most recent handle.
    `case-byzantine-vrf` starts two and stops them with one call from its EXIT trap."""
    pool = SpammerPool(dry=False)
    pool.p.run_ok = lambda *_a, **_k: True
    pool.start("k1", "0xa", "http://rpc")
    pool.start("k2", "0xb", "http://rpc")
    assert pool.running == 2
    pool.stop()
    assert pool.running == 0 and pool.last is None


def test_stop_is_idempotent():
    """It runs from an EXIT trap, where a second call (or a call with nothing started) must not
    turn a passing case into a failing one."""
    pool = SpammerPool(dry=True)
    pool.stop()
    pool.stop()
    assert pool.running == 0


def test_a_dry_pool_starts_nothing():
    """A thread that fired a `cast send` under `--dry-run` would be a live transaction from a dry
    run."""
    pool = SpammerPool(dry=True)
    assert pool.start("k", "0xa", "http://rpc") is None
    assert pool.running == 0


# ══ the rotation bring-up ═══════════════════════════════════════════════════════════════

def _bringup(dry=True, **kw):
    runner = Runner(dry=dry)
    return PP.RotationBringUp(runner=runner, label=kw.pop("label", "smoke-x"),
                              contracts_dir=kw.pop("contracts_dir", "/contracts"),
                              manifest=kw.pop("manifest", "/contracts/deployments/m.json"), **kw)


def _prod_ctx():
    return prod.ProdCtx(_bringup(dry=False))


def test_the_label_is_required():
    """`PP_ROT_LABEL` is `${PP_ROT_LABEL:?…}` — bash ABORTS on an unset one. It names every FAIL
    line, and five cases share this bring-up, so a default would make their failures
    indistinguishable."""
    with pytest.raises(ValueError):
        PP.RotationBringUp(runner=Runner(dry=True), label="")


def test_the_bring_up_phases_run_in_bash_order():
    """The 14 phases, as the ordered note sequence. Order is the assertion, not the contents:
    `setBlsVerifier` before `setConsensusKeys` (the keys are PoP-verified on the way in, so with no
    verifier the first setConsensusKeys reverts), the activation gov AFTER the keys, and the
    cold-restart last."""
    bu = _bringup()
    bu.run()
    notes = [i.note for i in bu.p.log if i.note]
    order = [n for n in ["phaseA-up", "spammer-key", "fund-spammer", "deploy-token",
                         "deploy-verifier", "token-transfer", "DeployStaking",
                         "setConsensusKeys-v0", "dpos-cold-restart"] if n in notes]
    assert order == ["phaseA-up", "spammer-key", "fund-spammer", "deploy-token",
                     "deploy-verifier", "token-transfer", "DeployStaking",
                     "setConsensusKeys-v0", "dpos-cold-restart"]
    assert notes.index("deploy-verifier") < notes.index("setConsensusKeys-v0")


def test_the_verifier_gov_precedes_the_consensus_keys():
    bu = _bringup()
    bu.run()
    argvs = [" ".join(i.argv) for i in bu.p.log]
    verifier_at = next(i for i, a in enumerate(argvs) if "setBlsVerifier(address)" in a)
    keys_at = next(i for i, a in enumerate(argvs) if "setConsensusKeys(address" in a)
    assert verifier_at < keys_at


def test_compose_file_is_re_exported_at_the_cold_restart(monkeypatch):
    """lib.sh:1348 — everything up to the cold restart runs against the BARE compose file, and
    from the restart on against the PAIR. It goes to os.environ AND the Runner env: the bare
    `docker compose exec` READERS inherit only os.environ, and a Runner env seeded earlier would
    otherwise shadow it."""
    monkeypatch.delenv("COMPOSE_FILE", raising=False)
    bu = _bringup()
    exports = []
    seen = []

    original = bu._set_compose

    def spy(value):
        exports.append(value)
        original(value)
        seen.append((os.environ.get("COMPOSE_FILE"), bu.p.env.get("COMPOSE_FILE")))

    bu._set_compose = spy
    bu.run()
    assert exports == [PP.PRODUCTION_BASE,
                       f"{PP.PRODUCTION_BASE}:{PP.PRODUCTION_DPOS_OVERLAY}"]
    assert all(env == runner_env == val for (env, runner_env), val in zip(seen, exports))


def test_the_activation_block_is_computed_on_the_literal_64_grid():
    """lib.sh:1320 — `ACT=$(( ((HEAD / 64) + 2) * 64 ))`. NOT the epoch interval: `EPOCH_LEN` is
    not read until the last line of the function, so the bash could not have used it. Substituting
    the real interval would move the block the whole case anchors on."""
    assert V.activation_block(0) == 128
    assert V.activation_block(100) == 192
    assert V.activation_block(128) == 256
    assert V.activation_block(100, grid=32) != V.activation_block(100, grid=64)


def test_epoch_first_block_reads_the_bring_ups_two_globals():
    """`epoch_first_block` is defined at FILE scope in bash (lib.sh:1362), not inside
    `pp_bring_up_rotation`, precisely so it survives the helper's `local` scope."""
    bu = _bringup()
    bu.act, bu.epoch_len = 128, 32
    assert bu.epoch_first_block(0) == 128
    assert bu.epoch_first_block(3) == 224
    assert V.epoch_first_block(128, 32, 3) == 224


def test_six_containers_boot_but_five_get_consensus_keys():
    """`PP_VALS` is SIX and `PP_COMMITTEE_SIZE` is FIVE, and the difference IS the rotation these
    cases test: validator-5 boots as a follower and joins the committee later."""
    p = PP.ProductionPathProfile()
    assert len(p.committee()) == 6
    assert len(p.keyed_committee()) == 5
    assert "validator-5" in p.committee() and "validator-5" not in p.keyed_committee()


def test_the_committee_size_is_env_overridable(monkeypatch):
    """The n=6 byzantine repro sets `PP_COMMITTEE_SIZE=6`."""
    monkeypatch.setenv("PP_COMMITTEE_SIZE", "6")
    assert len(PP.ProductionPathProfile.from_env().keyed_committee()) == 6


def test_the_cold_restart_recreates_every_validator_plus_the_full_node():
    bu = _bringup()
    bu.run()
    argv = next(i.argv for i in bu.p.log if i.note == "dpos-cold-restart")
    assert argv[:5] == ["docker", "compose", "up", "-d", "--force-recreate"]
    assert argv[5:] == [f"validator-{i}" for i in range(6)] + ["full-node"]


def test_the_profile_drives_compose_through_the_environment():
    """Unlike the STATIC profile, which names `-f` flags explicitly and leaves every bare command
    resolving `docker-compose.yml`. The five bash cases `export COMPOSE_FILE=…` before sourcing
    `lib.sh`, so this profile has a `compose_file_env` and the static one deliberately does not."""
    p = PP.ProductionPathProfile()
    assert p.compose_file_env("base") == PP.PRODUCTION_BASE
    assert p.compose_file_env("dpos").count(":") == 1
    from dpos_harness.stack.profiles import StaticProfile
    assert StaticProfile().compose_file_env("dpos") is None


def test_the_read_set_is_six_validators_plus_the_full_node(monkeypatch):
    """`_read_pp_nodes` (lib.sh:654-662). validator-0 and the full-node over their HOST ports,
    validator-1..5 by `docker compose exec` — the split comes from `topology.HOST_RPC_PORTS`, the
    same map the compose files publish from."""
    monkeypatch.setattr("dpos_harness.core.nodes.check_external", lambda port: f"host:{port}")
    monkeypatch.setattr("dpos_harness.core.nodes.check_node", lambda svc: f"exec:{svc}")
    got = PP.ProductionPathProfile().read_nodes()
    assert len(got) == 7
    assert got[0][1] == "host:8545"
    assert got[-1][1] == "host:18545"
    assert [v for _, v in got[1:6]] == [f"exec:validator-{i}" for i in range(1, 6)]


# ══ the create-nonce drift detector ════════════════════════════════════════════════════

def test_staking_reader_agreement_is_case_insensitive():
    """The manifest is checksummed and the reader file is not, so a byte comparison would report
    three mismatches on a perfectly aligned deploy."""
    pre = {"staking_address": "0xAABB", "chain_config_address": "0xccdd",
           "liveness_slashing_address": "0xEEFF"}
    manifest = {"staking": "0xaabb", "chain_config": "0xCCDD", "liveness_slashing": "0xeeff"}
    assert V.staking_reader_mismatches(pre, manifest) == []


def test_a_drifted_staking_reader_names_every_mismatch():
    """The detector's whole job: a deployer nonce that moved makes the pre-written file point at
    an address DeployStaking did not deploy, and the run would then read a codeless contract."""
    pre = {"staking_address": "0x1111", "chain_config_address": "0xccdd",
           "liveness_slashing_address": "0x3333"}
    manifest = {"staking": "0x2222", "chain_config": "0xccdd", "liveness_slashing": "0x4444"}
    got = V.staking_reader_mismatches(pre, manifest)
    assert [k for k, _, _ in got] == ["staking_address", "liveness_slashing_address"]


def test_a_missing_reader_key_is_a_mismatch_not_a_skip():
    """One absent key means the generator changed, which is exactly the drift this gate is for."""
    got = V.staking_reader_mismatches({}, {"staking": "0xaa", "chain_config": "0xbb",
                                           "liveness_slashing": "0xcc"})
    assert len(got) == 3


def test_the_bring_up_fails_loud_on_a_drifted_staking_reader(monkeypatch, tmp_path):
    """And the message names the FIX, as bash's does — a mismatch with no mention of
    `--staking-reader-create-nonces` leaves the reader with two addresses and no idea what
    produced the difference."""
    manifest = tmp_path / "m.json"
    manifest.write_text('{"staking":"0xaa","chain_config":"0xbb","governance":"0xcc",'
                        '"liveness_slashing":"0xdd"}')
    bu = _bringup(dry=False, manifest=str(manifest))
    bu.staking_rt, bu.chain_config_rt, bu.liveness_rt = "0xaa", "0xbb", "0xdd"
    chain = Chain(runner=bu.p)
    chain.runtime_cat = lambda _p: '{"staking_address":"0xZZ","chain_config_address":"0xbb",' \
                                   '"liveness_slashing_address":"0xdd"}'
    with pytest.raises(PP.RotationBringUpError) as e:
        bu._assert_staking_reader(chain)
    assert "staking-reader-create-nonces" in e.value.message


def test_a_manifest_missing_an_address_is_named():
    """bash asserts each of the four is `0x…` and `cat`s the manifest on failure. A jq miss returns
    `null`, which is what this catches."""
    assert V.manifest_missing({"staking": "0xa", "chain_config": "0xb", "governance": "0xc",
                               "liveness_slashing": "0xd"}) == []
    assert V.manifest_missing({"staking": "0xa"}) == ["CHAIN_CONFIG_RT", "GOV_ADDR",
                                                      "LIVENESS_RT"]
    assert V.manifest_missing({}) == ["STAKING_RT", "CHAIN_CONFIG_RT", "GOV_ADDR", "LIVENESS_RT"]


# ══ the governance wait, as a pure classifier ══════════════════════════════════════════

@pytest.mark.parametrize("state,head,end,stalled,verdict", [
    ("4", 5, 10, 0, "succeeded"),
    ("3", 5, 10, 0, "terminal"),     # Defeated — named, never timed out
    ("2", 5, 10, 0, "terminal"),     # Canceled
    ("6", 5, 10, 0, "terminal"),     # Expired
    ("1", 20, 10, 0, "deadline"),    # past end+5 and still Active
    ("1", 14, 10, 0, "waiting"),     # end+5 exactly — the margin is inclusive of 15
    ("1", 5, 10, 999, "frozen"),
    ("1", 5, 10, 0, "waiting"),
    ("1", 5, None, 0, "waiting"),    # unreadable deadline is NOT a failure
    ("1", 5, None, 999, "frozen"),   # ...but the frozen escape still bounds it
])
def test_gov_wait_verdicts(state, head, end, stalled, verdict):
    got, msg = V.gov_wait_verdict(state, head, end, stalled, desc="d")
    assert got == verdict
    assert (msg == "") == (verdict in ("succeeded", "waiting"))


def test_a_terminal_verdict_names_the_state_rather_than_a_timeout():
    """bash: "proposal Defeated (state=3)". Reporting a timeout for a proposal that LOST sends the
    reader looking at chain throughput."""
    _, msg = V.gov_wait_verdict("3", 5, 10, 0, desc="setBlsVerifier")
    assert "Defeated" in msg and "state=3" in msg and "setBlsVerifier" in msg


@pytest.mark.parametrize("byte,name", [(0, "Pending"), (1, "Active"), (3, "Defeated"),
                                       (4, "Succeeded"), (7, "Executed"), (8, "Unknown"),
                                       (-1, "Unknown"), ("x", "Unknown"), (None, "Unknown")])
def test_gov_state_names(byte, name):
    assert V.gov_state_name(byte) == name


def test_the_vote_freeze_is_not_the_execute_confirm_knob(monkeypatch):
    """`pp_gov_wait_succeeded`'s frozen-chain escape is a LITERAL 120 s in bash (lib.sh:1038) and
    deliberately NOT the gov-confirm freeze knob, which bounds the post-execute nonce confirm.
    Wiring the knob to both lets a caller widening the confirm budget silently widen the vote wait
    too."""
    from dpos_harness.chain import writes
    monkeypatch.setenv("SIM_GOV_CONFIRM_FROZEN_S", "999")
    chain, _ = _chain()
    assert chain.gov_confirm_frozen_s == 999
    assert writes.GOV_VOTE_FROZEN_S == 120


# ══ the dry-run transcript is complete ═════════════════════════════════════════════════

def test_the_dry_transcript_walks_every_phase():
    """A dry run must walk the choreography ONCE and record every gate, including the WAITS — a
    transcript that jumps from the `up` to the next write says nothing about what happens between
    them, and the waits are where this bring-up hangs."""
    bu = _bringup()
    bu.run()
    steps = [(i.argv[1], i.note) for i in bu.p.log if i.kind == "step"]
    labels = [s for s, _ in steps]
    assert labels.count("export") == 2                 # base, then the pair
    assert labels.count("poll") == 4                   # 3 converges + the activation wait
    assert any("check_external" in n for _, n in steps)


def test_the_dry_transcript_issues_no_read_that_would_gate_it():
    """Dry mode answers reads with canned values, so a gate computed over them is meaningless. The
    bring-up must still reach its last phase — an `epoch_len` of 0 read off an empty dry stdout
    would abort the transcript one phase early."""
    bu = _bringup()
    bu.run()
    assert bu.epoch_len == PP.ACTIVATION_GRID
    assert bu.act == 128
