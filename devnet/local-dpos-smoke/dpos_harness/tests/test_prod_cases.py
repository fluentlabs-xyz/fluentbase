"""The five production-path case DRIVERS: wiring, transcript ORDER, and the two seams they need.

The verdicts are covered in `test_prod_case_verdicts.py`. What is covered here is everything a
verdict cannot see — that the reads and writes happen, in the right order, against the right
services, with the background load still running underneath them.

THREE THINGS THIS FILE EXISTS FOR:

  * **The spammer is a background thread and nothing in a case mentions it** (§2.4 item 1, risk
    R5). Six cases depend on the tx pressure it applies and every assertion still passes without
    it — the chain simply has nothing to finalize under. So the tests below assert that the
    bring-up STARTS one and that the cleanup reaps it, on both the passing and the failing path.
  * **Order is the evidence.** A restart issued before the corruption it is meant to expose, or a
    kill issued before the gate that establishes the window, is a different test that still passes.
    Each destructive case's write sequence is pinned as an ordered list of notes.
  * **The two seams `case-byzantine-vrf` needs.** Its bash builds a bring-up of its own; routing it
    through the shared one is only safe if the overlay reaches the COLD RESTART and not phase A,
    and if its five extra transfers land after DeployStaking and before the first governance write.
    Both are pinned here, because both are silent when wrong: an overlay applied to phase A builds
    a stack the bash never builds, and a transfer sent early shifts DeployStaking's CREATE
    addresses off the prediction baked into `staking-reader.json`.
"""

from __future__ import annotations

import importlib
import io
import pathlib
import sys

import pytest

from dpos_harness import cli
from dpos_harness.cases.smoke import asserts_byzantine_vrf as B
from dpos_harness.cases.smoke import byzantine_vrf, prod, verdicts_rotation as VR
from dpos_harness.core.proc import Runner
from dpos_harness.stack import production_path as PP

#: The five, and the module each maps to in `cli.CASES`.
PROD_CASES = {
    "smoke-production-path": "smoke.production_path",
    "smoke-vrf-rotation": "smoke.vrf_rotation",
    "smoke-vrf-dkg-halt": "smoke.vrf_dkg_halt",
    "smoke-vrf-dkg-durability": "smoke.vrf_dkg_durability",
    "smoke-byzantine-vrf": "smoke.byzantine_vrf",
}

SMOKE_DIR = pathlib.Path(__file__).resolve().parents[2]


def _dry(case: str):
    """Run one case under `--dry-run` and return `(rc, transcript)`."""
    mod = importlib.import_module(f"dpos_harness.cases.{PROD_CASES[case]}")
    buf = io.StringIO()
    saved, sys.stdout = sys.stdout, buf
    try:
        rc = mod.run_case(["--dry-run"])
    finally:
        sys.stdout = saved
    return rc, buf.getvalue()


def _notes(transcript: str):
    """The ordered `# note` comments of the recorded commands — the write choreography."""
    out = []
    for line in transcript.splitlines():
        if "    # " in line:
            out.append(line.rsplit("    # ", 1)[1].strip())
    return out


@pytest.fixture(scope="module")
def transcripts():
    return {c: _dry(c) for c in PROD_CASES}


# ══ wiring ═════════════════════════════════════════════════════════════════════════════════

@pytest.mark.parametrize("case,module", sorted(PROD_CASES.items()))
def test_every_case_is_registered_and_importable(case, module):
    assert cli.CASES[case] == module
    assert callable(importlib.import_module(f"dpos_harness.cases.{module}").run_case)


@pytest.mark.parametrize("case", sorted(PROD_CASES))
def test_the_makefile_carries_a_py_target_and_declares_it_phony(case):
    """`-py` targets are ADDED beside the bash ones (chunks 0-4 set that), never replacing them:
    the port is proven only once both have run against the same stack and their verdicts have been
    compared, and that comparison needs both to stay runnable."""
    text = (SMOKE_DIR / "Makefile").read_text()
    assert f"\n{case}-py:" in text, f"{case}-py target missing"
    assert f"python3 -m dpos_harness case {case} $(ARGS)" in text
    assert f"{case}-py" in text.split("\n\nregen-contracts")[0], f"{case}-py not in .PHONY"
    assert f"\n{case}:" in text, "the bash target must still be there"


def test_the_aggregate_dry_run_target_covers_all_five():
    text = (SMOKE_DIR / "Makefile").read_text()
    block = text.split("smoke-prod-dry-run:")[1].split("\n\n")[0]
    for case in PROD_CASES:
        assert case in block, case


@pytest.mark.parametrize("case", sorted(PROD_CASES))
def test_a_dry_run_walks_the_whole_choreography_and_passes(case, transcripts):
    rc, text = transcripts[case]
    assert rc == 0, text[-2000:]
    assert "# dpos_harness " + case in text
    assert text.rstrip().endswith("commands")
    # The bring-up alone is ~90 commands; a case that fell over early would be far shorter.
    assert int(text.rstrip().rsplit("# ", 1)[1].split()[0]) > 120


@pytest.mark.parametrize("case", sorted(PROD_CASES))
def test_an_unknown_argument_is_refused(case):
    mod = importlib.import_module(f"dpos_harness.cases.{PROD_CASES[case]}")
    assert mod.run_case(["--wat"]) == 2


# ══ the background spammer (§2.4 item 1, risk R5) ══════════════════════════════════════════

@pytest.mark.parametrize("case", sorted(PROD_CASES))
def test_every_case_starts_the_tx_spammer_before_the_deploys(case, transcripts):
    """`pp_spammer_start` is the ONLY background job in the bash tree and no case mentions it. It
    starts BEFORE the deploys so user tx pressure is on the mempool across every transition the
    case then measures — a case that lost it keeps passing and stops measuring anything."""
    _, text = transcripts[case]
    notes = _notes(text)
    assert "spammer-key" in notes and "fund-spammer" in notes
    assert notes.index("fund-spammer") < notes.index("DeployStaking")


def test_the_spammer_is_reaped_on_the_PASSING_path(monkeypatch, tmp_path):
    """A leaked spammer keeps sending against a torn-down RPC forever — the 2026-07-14 3.5-hour
    orphan incident. `RotationBringUp` deliberately does NOT stop its own, because the trap covers
    the ASSERTION phase too."""
    seen = _spammer_spy(monkeypatch)
    rc = prod.run("smoke-x", [lambda ctx: None], argv=["--dry-run"],
                  manifest=str(tmp_path / "m.json"))
    assert rc == 0
    assert seen["stopped"] == 1


def test_the_spammer_is_reaped_on_the_FAILING_path(monkeypatch, tmp_path):
    """bash's `exit 1` TERMINATED the process, which fired the trap; a Python `raise` only unwinds.
    The `finally` is that re-wiring, and it has to cover a failure raised from inside a PHASE."""
    seen = _spammer_spy(monkeypatch)

    def boom(_ctx):
        raise prod.ProdFailure("smoke-x", "deliberate")

    assert prod.run("smoke-x", [boom], argv=["--dry-run"],
                    manifest=str(tmp_path / "m.json")) == 1
    assert seen["stopped"] == 1


@pytest.mark.parametrize("preset", [None, "docker-compose.yml"])
def test_a_case_puts_COMPOSE_FILE_back_the_way_it_found_it(monkeypatch, tmp_path, preset):
    """The bring-up exports `COMPOSE_FILE` into `os.environ` — the bare `docker compose exec`
    readers inherit only that. bash's process exits at the end of a case so nothing observes the
    leftover; here a second case, or the harness's own test suite, would inherit a compose file
    pointing at the PREVIOUS run's project. Restored after the teardown, which still needs it.

    Found by this suite: running a production-path case in-process broke
    `test_static_stack.py::test_bring_up_never_exports_compose_file` two files later."""
    import os
    if preset is None:
        monkeypatch.delenv("COMPOSE_FILE", raising=False)
    else:
        monkeypatch.setenv("COMPOSE_FILE", preset)
    prod.run("smoke-x", [lambda ctx: None], argv=["--dry-run"],
             manifest=str(tmp_path / "m.json"))
    assert os.environ.get("COMPOSE_FILE") == preset


def _spammer_spy(monkeypatch):
    seen = {"stopped": 0}
    original = prod.SpammerPool.stop

    def spy(self):
        seen["stopped"] += 1
        return original(self)

    monkeypatch.setattr(prod.SpammerPool, "stop", spy)
    return seen


# ══ the destructive write ORDER ════════════════════════════════════════════════════════════

def test_the_halt_kills_PRE_SEAL_then_RESTARTS_and_only_then_measures(transcripts):
    """The restart is the whole design: on n=5 the DKG dealer-quorum and the consensus quorum are
    both 4, so KEEPING the two down stalls consensus BELOW the boundary and the `skipping propose`
    line never fires. Restarting restores consensus quorum so the chain climbs to the boundary,
    where the DKG-None skip wedges it. Stop → sleep → start, in that order and nothing between."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    notes = _notes(text)
    assert "halt-preseal-stop" in notes and "halt-preseal-start" in notes
    assert notes.index("halt-preseal-stop") < notes.index("halt-preseal-start")
    body = text.split("halt-preseal-stop")[1].split("halt-preseal-start")[0]
    assert "[step] sleep  (3s)" in body


def test_the_halt_TEARS_both_journals_while_both_victims_are_STOPPED(transcripts):
    """The premise the case used to rest on is GONE. `maybe_start`'s `JournalLoad::Present` arm
    re-derives the seeded dealer while `last_height < epoch_start − DKG_MARGIN_BLOCKS`
    (`beacon/actor.rs:1046-1054`), so a victim merely stopped and restarted before the seal
    deadline SEALS NORMALLY and the ceremony finalizes — which is why the case failed. Only the
    `Torn` arm sits a member out, so the journals must actually be torn.

    BOTH tears land inside the SAME stop window: restarting v_k0 before v_k1's journal was torn
    would give it time to reach the deadline and seal, putting a fourth dealer log back in the
    count."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    notes = _notes(text)
    tears = [n for n in notes if n.startswith("halt-")]
    assert tears == ["halt-preseal-stop",
                     "halt-v1-rm-share", "halt-v1-tear-journal", "halt-v1-verify-tear",
                     "halt-v2-rm-share", "halt-v2-tear-journal", "halt-v2-verify-tear",
                     "halt-preseal-start"]


def test_the_halt_removes_the_SHARE_before_it_tears_the_journal(transcripts):
    """`maybe_start` returns at `actor.rs:982-989` on `store.contains_key(&target)` — BEFORE
    `load_journal`. A finalized share short-circuits the whole thing, the torn journal is never
    read, and the victim rejoins the ceremony as if nothing happened."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    notes = _notes(text)
    for v in (1, 2):
        assert notes.index(f"halt-v{v}-rm-share") < notes.index(f"halt-v{v}-tear-journal")


def test_the_halt_tear_goes_through_a_THROWAWAY_container_and_writes_OCTAL(transcripts):
    """`docker compose exec` on a STOPPED container SUCCEEDS and does nothing, and genesis-init's
    `/bin/sh` has POSIX `\\ooo` but not the bash/coreutils `\\xHH` — either mistake leaves the
    journal intact (or writes literal characters), the `Torn` arm never fires, and the case
    measures a node that was never torn."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    tear_lines = [ln for ln in text.splitlines() if "-tear-journal" in ln and "halt-v" in ln]
    assert len(tear_lines) == 2
    for line in tear_lines:
        assert "docker compose run --rm --no-deps -T --entrypoint sh genesis-init" in line
        assert " exec " not in line
        assert '\\377\\377\\377\\377' in line and "\\x" not in line
        assert "conv=notrunc" in line and "count=4" in line


def test_the_halt_PROVES_the_sit_out_before_it_measures_the_freeze(transcripts):
    """ANTI-VACUITY, and the reason this case is not "assert the chain stopped". A chain frozen for
    an unrelated reason — a lost peer, a crashed victim, a consensus stall below the boundary —
    would otherwise read as a pass. So the readback that proves the corruption landed and the poll
    for the `Torn` arm both come BEFORE the 30 s freeze window, and the freeze is only ever
    interpreted as a halt of a committee already proven shareless."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    assert text.index("halt-v2-verify-tear") < text.index("[step] sleep  (30s)")
    after_start, freeze = text.split("halt-preseal-start", 1)[1].split("[step] sleep  (30s)", 1)
    # The Torn-arm poll for BOTH victims, and its log reads, sit in that stretch.
    assert after_start.count("[step] poll  (<lambda> (<= 120s))") == 2
    assert "logs_all(validator-1)" in after_start and "logs_all(validator-2)" in after_start
    assert VR.TORN_POLL_S == 120
    assert freeze  # the freeze window really is downstream of all of it


def test_the_halt_gate_precedes_the_kill(transcripts):
    """The pre-seal gate establishes the window; killing before it would be a kill at an unknown
    seal state, which is the difference between this case and the durability one."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    assert text.index("both_preseal") < text.index("halt-preseal-stop")


def test_the_halt_freeze_windows_are_the_BASH_ones(transcripts):
    """§2.4 item 3: this case PASSES BY TIMEOUT. A chain that has not produced a block yet is
    indistinguishable from one that never will over a short enough window, so shortening either
    window leaves the case green and testing nothing."""
    assert (VR.FREEZE_WINDOW_S, VR.REFREEZE_WINDOW_S) == (30, 15)
    _, text = transcripts["smoke-vrf-dkg-halt"]
    assert "[step] sleep  (30s)" in text and "[step] sleep  (15s)" in text


def test_the_halt_measures_the_TIP_and_not_finalized(transcripts):
    """With the DKG outcome None the proposer declines to BUILD the boundary block, so the TIP is
    what freezes — finalized could still be climbing toward it from below. A freeze check on
    finalized would report progress on a chain that is permanently wedged."""
    _, text = transcripts["smoke-vrf-dkg-halt"]
    before_freeze, after_freeze = text.split("halt-preseal-start")[1].split(
        "[step] sleep  (30s)", 1)
    # The window is BRACKETED by two tip reads, and the whole climb before it is measured on the
    # tip as well. A single finalized read anywhere in that stretch would be the silent bug.
    assert before_freeze.rstrip().endswith("[step] read  (tip_dec())")
    assert after_freeze.lstrip().startswith("[step] read  (tip_dec())")
    assert "finalized_dec" not in before_freeze


def test_the_durability_control_stalls_BEFORE_it_restarts(transcripts):
    """The 12 s freeze with 2 of 5 down is the "recovery required" CONTROL. Restarting first would
    make the recovery afterwards prove nothing — the victims might never have been needed."""
    _, text = transcripts["smoke-vrf-dkg-durability"]
    notes = _notes(text)
    assert notes.index("phase1-stop") < notes.index("phase1-start")
    body = text.split("phase1-stop")[1].split("phase1-start")[0]
    assert "[step] sleep  (12s)" in body and VR.STALL_WINDOW_S == 12


def test_the_journal_is_torn_while_the_victim_is_STOPPED_and_verified_before_the_restart(
        transcripts):
    """FOUR ordering facts, and every one of them is silent when wrong:

      * the corruption lands between the STOP and the START — `docker compose exec` on a stopped
        container is a silent no-op, which is why it goes through the volume;
      * the share is removed BEFORE the journal is torn, so the restart reaches `load_journal`
        instead of the store-hit early return;
      * the readback VERIFIES the corruption before the node comes back — a no-op that was not
        caught here would leave every later assertion measuring a node that was never torn;
      * the restart is LAST."""
    _, text = transcripts["smoke-vrf-dkg-durability"]
    notes = _notes(text)
    order = [n for n in notes if n.startswith("phase3-")]
    assert order == ["phase3-stop", "phase3-rm-share", "phase3-tear-journal",
                     "phase3-verify-tear", "phase3-start"]


def test_the_torn_file_ops_go_through_a_THROWAWAY_container_never_exec(transcripts):
    """`docker compose exec` only works on a RUNNING container and exec-on-stopped SUCCEEDS while
    doing nothing — the bug the volume path replaced (`case-vrf-dkg-durability.sh:90-92`)."""
    _, text = transcripts["smoke-vrf-dkg-durability"]
    for line in text.splitlines():
        if "phase3-rm-share" in line or "phase3-tear-journal" in line \
                or "phase3-verify-tear" in line:
            assert "docker compose run --rm --no-deps -T --entrypoint sh genesis-init" in line
            assert " exec " not in line


def test_the_corruption_writes_OCTAL_escapes():
    """genesis-init's `/bin/sh` supports POSIX `\\ooo` octal; `printf \\xHH` hex is a
    bash/coreutils-only extension and would write the literal characters instead of the four 0xff
    bytes — a corruption that lands as NoFile rather than Torn, i.e. the opposite experiment."""
    _, text = _dry("smoke-vrf-dkg-durability")
    line = next(ln for ln in text.splitlines() if "phase3-tear-journal" in ln)
    assert '\\377\\377\\377\\377' in line and "\\x" not in line
    assert "conv=notrunc" in line and "count=4" in line


def test_the_production_path_stops_the_victim_only_after_syncing_to_an_epoch_start(transcripts):
    """The stop is synced to an epoch START so the victim is shareless for the WHOLE participation
    window (seen=0 ≪ floor), which maximises the below-floor margin. Stopping mid-epoch would
    leave it partially credited and the jail might not fire at all."""
    _, text = transcripts["smoke-production-path"]
    before = text.split("liveness-eject-stop")[0]
    assert before.count("read  (current_epoch())") >= 2


# ══ the two byzantine-vrf seams ════════════════════════════════════════════════════════════

def test_the_byzantine_overlay_reaches_the_COLD_RESTART_and_not_phase_A(transcripts):
    """Phase A is the BARE genesis chain; bash exports the bare name at `:85` and only widens it at
    the restart (`:288`). An overlay applied to the phase-A `up --build` would build a stack the
    bash never builds."""
    _, text = transcripts["smoke-byzantine-vrf"]
    exports = [ln for ln in text.splitlines() if "COMPOSE_FILE=" in ln]
    assert exports[0].endswith("COMPOSE_FILE=docker-compose.production-path.yml)")
    assert exports[1].endswith("docker-compose.production-path.yml:"
                               "docker-compose.production-path.dpos.yml:"
                               "docker-compose.byzantine-vrf.yml)")


def test_the_byzantine_overlay_file_exists_and_flags_the_configured_node():
    """The overlay hardcodes ONE service. A `BYZ_VRF_IDX` pointing anywhere else would flag
    nothing — compose would ignore a service that is not in the file — and the case would spend its
    whole budget waiting for a forge that cannot happen."""
    text = (SMOKE_DIR / byzantine_vrf.BYZANTINE_OVERLAY).read_text()
    assert "FLUENT_DPOS_BYZANTINE: forge-beacon-pk" in text
    assert f"validator-{VR.DEFAULT_BYZ_IDX}:" in text


def test_the_other_four_cases_carry_no_overlay(transcripts):
    for case in PROD_CASES:
        if case == "smoke-byzantine-vrf":
            continue
        _, text = transcripts[case]
        assert "overlays=[]" in text.splitlines()[0], case
        assert "byzantine-vrf.yml" not in text, case


def test_the_extra_transfers_land_AFTER_DeployStaking_and_BEFORE_the_first_governance_write(
        transcripts):
    """The load-bearing half of the second seam. Sending them earlier would advance the deployer
    nonce and shift DeployStaking's CREATE addresses off the prediction baked into
    `staking-reader.json`, so the node would read ChainConfig at the wrong address and the `--dpos`
    cold start would fail."""
    _, text = transcripts["smoke-byzantine-vrf"]
    notes = _notes(text)
    deploy = notes.index("DeployStaking")
    first_gov = notes.index("gov-propose")
    for note in ("toggle-key", "toggle-addr", "fund-toggle-delegator"):
        assert deploy < notes.index(note) < first_gov, note
    # SIX BLEND transfers: the shared bring-up's one to the joiner, then the hook's five — the
    # byzantine owner, the toggle delegator, and the three floor-bumped owners (v1/v3/v4; the
    # byzantine is skipped there because it already got its own).
    transfers = [ln for ln in text.splitlines() if "# token-transfer" in ln]
    assert len(transfers) == 6
    assert VR.BYZ_BLEND_WEI in transfers[1] and VR.TOGGLE_BLEND_WEI in transfers[2]
    assert all(VR.FLOOR_BLEND_WEI in ln for ln in transfers[3:])


def test_the_stake_setup_boosts_the_byzantine_and_lifts_the_other_floor(transcripts):
    """(i) the byzantine permanently in the top-5 so it holds a share at every boundary; (ii) the
    joiner the only clean swing member, which needs the OTHER 1e18 validators lifted above its
    self-stake floor or a joiner-OUT ties them."""
    _, text = transcripts["smoke-byzantine-vrf"]
    notes = _notes(text)
    assert "delegate-byz-boost" in notes
    bumped = [n for n in notes if n.startswith("delegate-floor-bump-")]
    assert bumped == ["delegate-floor-bump-v1", "delegate-floor-bump-v3", "delegate-floor-bump-v4"]
    assert VR.BYZ_BOOST_WEI in text and VR.FLOOR_BUMP_WEI in text


def test_the_toggles_come_from_the_dedicated_delegator_not_the_joiners_own_key(transcripts):
    """An `undelegate` from the joiner's owner key would trip `OwnerSelfStakeBelowMinimum`, so the
    OUT half of the drive would fail on every iteration."""
    _, text = transcripts["smoke-byzantine-vrf"]
    toggle = next(ln for ln in text.splitlines() if "# toggle-in-v5" in ln)
    approve = next(ln for ln in text.splitlines() if "# approve-toggle-delegator" in ln)
    key = approve.split("--private-key ")[1].split()[0]
    assert toggle.split("--private-key ")[1].split()[0] == key
    assert VR.V5_IN_AMOUNT in toggle


def test_the_drive_watches_the_forge_count_before_and_after_every_flip(transcripts):
    """The loop watches three things in one poll — the forge count, the committee timeline and
    whether THIS flip landed. Splitting them would triple the RPC load on a chain the case is also
    measuring, and dropping the pre-flip read would issue a toggle after the forge already fired."""
    _, text = transcripts["smoke-byzantine-vrf"]
    counts = [i for i, ln in enumerate(text.splitlines()) if VR.FORGE_LINE in ln
              and "log_count" in ln]
    toggle = next(i for i, ln in enumerate(text.splitlines()) if "# toggle-in-v5" in ln)
    assert any(i < toggle for i in counts) and any(i > toggle for i in counts)


def test_the_drive_reads_its_knobs_from_the_environment(monkeypatch):
    monkeypatch.setenv("BYZ_VRF_IDX", "3")
    monkeypatch.setenv("BYZ_VRF_MAX_TOGGLES", "9")
    monkeypatch.setenv("BYZ_VRF_BUDGET_MIN", "12")
    drive = B.ByzantineDrive()
    assert (drive.byz_idx, drive.max_toggles, drive.budget_min) == (3, 9, 12)
    assert drive.service == "validator-3"
    assert "validator-3" not in drive.honest_nodes


def test_the_honest_set_is_every_deriving_node_but_the_byzantine():
    from dpos_harness.cases.smoke.asserts_prod import PROD_NODES
    drive = B.ByzantineDrive(byz_idx=2)
    assert set(drive.honest_nodes) == set(PROD_NODES) - {"validator-2"}
    assert len(drive.honest_nodes) == len(PROD_NODES) - 1


# ══ the profile seam ═══════════════════════════════════════════════════════════════════════

def test_overlays_ride_the_dpos_phase_only():
    p = PP.ProductionPathProfile(extra_overlays=("x.yml",))
    assert p.compose_files("base") == (PP.PRODUCTION_BASE,)
    assert p.compose_files("dpos") == (PP.PRODUCTION_BASE, PP.PRODUCTION_DPOS_OVERLAY, "x.yml")
    assert p.compose_file_env("dpos").endswith(":x.yml")
    assert "x.yml" not in p.compose_file_env("base")


def test_a_profile_without_overlays_is_unchanged():
    """Four of the five cases pass none, and their compose environment must stay byte-identical to
    what chunk 5a shipped."""
    assert PP.ProductionPathProfile().compose_files("dpos") == (
        PP.PRODUCTION_BASE, PP.PRODUCTION_DPOS_OVERLAY)


def test_the_post_manifest_hook_fires_once_after_the_manifest_and_before_the_verifier_gov():
    """The seam's contract, pinned independently of the byzantine case that uses it: the hook needs
    a `Chain` (so it must run after the manifest is read) and its writes must precede the first
    governance action (so the deployer's tx sequence matches the bash)."""
    seen = []
    bu = PP.RotationBringUp(runner=Runner(dry=True), label="smoke-x", contracts_dir="/c",
                            manifest="/c/m.json",
                            post_manifest=lambda b: seen.append(len(b.p.log)))
    bu.run()
    assert len(seen) == 1
    notes = [i.note for i in bu.p.log if i.note]
    at = seen[0]
    before = [i.note for i in bu.p.log[:at] if i.note]
    assert "DeployStaking" in before
    assert "gov-propose" not in before
    assert "gov-propose" in notes


def test_no_hook_means_no_extra_commands():
    plain = PP.RotationBringUp(runner=Runner(dry=True), label="smoke-x", contracts_dir="/c",
                               manifest="/c/m.json")
    plain.run()
    hooked = PP.RotationBringUp(runner=Runner(dry=True), label="smoke-x", contracts_dir="/c",
                                manifest="/c/m.json", post_manifest=lambda b: None)
    hooked.run()
    assert len(plain.p.log) == len(hooked.p.log)


# ══ the reader collision that would have been silent ═══════════════════════════════════════

def test_finalized_and_tip_are_two_DIFFERENT_readers():
    """The five bash cases each define a LOCAL `head_dec()` that is `eth_blockNumber` — the
    speculative TIP — while `lib.sh`'s `finalized_dec` is the finalized head, and they use both in
    the same file for different questions. A ctx that answered one name with the other would
    silently convert three tip-based negative assertions (the beacon growth probe and both of the
    halt case's freeze windows) into finalized-based ones, which pass for the wrong reason."""
    ctx = prod.ProdCtx(PP.RotationBringUp(runner=Runner(dry=True), label="smoke-x",
                                          contracts_dir="/c", manifest="/c/m.json"))
    assert hasattr(ctx, "finalized_dec") and hasattr(ctx, "tip_dec")
    assert not hasattr(ctx, "head_dec"), "the ambiguous name must not come back"
    ctx.finalized_dec()
    ctx.tip_dec()
    markers = [i.note or " ".join(i.argv) for i in ctx.p.log]
    assert any("finalized_dec" in m for m in markers)
    assert any("tip_dec" in m for m in markers)
