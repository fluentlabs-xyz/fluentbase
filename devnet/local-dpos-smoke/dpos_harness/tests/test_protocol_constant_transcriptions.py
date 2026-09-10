"""The Python transcriptions of Rust protocol constants, against the Rust.

Two numbers in this tree are typed out in Python because Python cannot import a Rust `const`:
`compose_gen.MAX_COMMITTEE_SIZE` (the cap the compose generator refuses N above) and
`floor_halt_case.MIN_COMMITTEE_LENGTH` (the floor that case drives the population below). Both
are copies of `crates/types/src/staking_protocol.rs`.

`scripts/xp/agreement_check.py` (G8) compares the first of them, but the checker is a manual
script and the harness's own tests are what CI runs — and the two tests that use the cap
(`test_compose_gen`, `test_topology`) both read it FROM `compose_gen`, so setting it to 60 keeps
them green while the stand generates a compose file the contract will reject at genesis. This
test is the one that says otherwise.

It SKIPS, loudly, when `crates/` is not in the tree (the harness is run from a copy that has
only `devnet/`), and fails when the file is there but the constant is not: a transcription
check that cannot find its source must not report agreement.
"""

from __future__ import annotations

import pathlib
import re

import pytest

from dpos_harness.stack import compose_gen

PROTOCOL_RS = pathlib.Path(__file__).resolve().parents[4] / "crates/types/src/staking_protocol.rs"
FLOOR_HALT = pathlib.Path(__file__).resolve().parents[2] / "scripts/xp/floor_halt_case.py"


def _rust_const(text: str, name: str) -> int:
    m = re.search(rf"pub const {name}:\s*\w+\s*=\s*([\d_]+);", text)
    assert m, f"{name} is no longer a plain integer `pub const` in {PROTOCOL_RS}"
    return int(m.group(1).replace("_", ""))


def _python_const(path: pathlib.Path, name: str) -> int:
    m = re.search(rf"^{name} = (\d+)\s*(?:#.*)?$", path.read_text(), re.M)
    assert m, f"{name} is no longer a module-level integer in {path}"
    return int(m.group(1))


@pytest.mark.parametrize("rust_name,py", [
    ("MAX_COMMITTEE_SIZE", "compose_gen"),
    ("MIN_COMMITTEE_LENGTH", "floor_halt_case"),
])
def test_the_python_copy_equals_the_rust_constant(rust_name, py):
    if not PROTOCOL_RS.is_file():
        pytest.skip(f"the node crates are not in this tree ({PROTOCOL_RS}) — the transcription "
                    "cannot be compared against anything, so this proves nothing rather than "
                    "passing")
    want = _rust_const(PROTOCOL_RS.read_text(), rust_name)
    if py == "compose_gen":
        got = compose_gen.MAX_COMMITTEE_SIZE
    else:
        got = _python_const(FLOOR_HALT, rust_name)
    assert got == want, (
        f"{py} transcribes {rust_name} as {got}; staking_protocol.rs declares {want}")
