"""dataroot.py — the two `SIM_DATA_ROOT` wipes (lib.sh:511-567).

`docker compose down -v` removes named-volume METADATA but never touches a BIND mount's host
contents. So the moment a run relocates `/runtime` onto a bind (`SIM_DATA_ROOT` set — the
disk-relocation and RAM-mode profiles both do), the fresh-state-each-run semantics the named
volume gave have to be restored by hand. Both bring-ups need this and both teardowns need it,
which is why it is here rather than a private method on one of them.

WHY A CONTAINER AND NOT `rm -rf`: reth writes the tree as ROOT, so a host-side `rm -rf` by the
invoking user hits Permission-denied on the root-owned subdirs and silently leaves the tree in
place. `data_root_wipe` clears the CONTENTS as root inside a throwaway busybox and then
VERIFIES the directory is empty IN THE SAME container — a docker-run failure (daemon down,
image pull) surfaces as a nonzero rc rather than a false "wiped". The directory itself is kept
so the bind mount stays valid.

WHY THE VERIFY MATTERS: this was once `|| true`. A wipe that did not actually clear the tree
leaves a leaked reth datadir, so owner-0 is already past nonce 0 at the next deploy, the
staking-address prediction drifts, and the run dies opaquely at bring-up (v54).

`ram_root_purge` is the other one, and it is deliberately NOT the same mechanism — see its
docstring.
"""

from __future__ import annotations

import os
import shutil

#: The env var naming the relocated data root. `""`/unset means "named volume", i.e. no-op.
DATA_ROOT_ENV = "SIM_DATA_ROOT"

#: The busybox one-liner: clear the contents (including dotfiles), then count what is left and
#: exit 3 if anything survived. Verification runs in the SAME container as the removal, so a
#: partially-failed wipe cannot report success.
_WIPE_SH = ("rm -rf /wipe/* /wipe/.[!.]* /wipe/..?* 2>/dev/null; "
            "n=$(ls -A /wipe 2>/dev/null | wc -l); [ \"$n\" = 0 ] || exit 3")


def data_root() -> str:
    """The configured data root with any trailing slash removed, or "" when unset."""
    return os.environ.get(DATA_ROOT_ENV, "").rstrip("/")


def _runtime_dir(root: str) -> str:
    return f"{root}/runtime"


def wipe_argv(root: str):
    """The `docker run --rm -v <root>/runtime:/wipe busybox sh -c '…'` argv."""
    return ["docker", "run", "--rm", "-v", f"{_runtime_dir(root)}:/wipe", "busybox", "sh", "-c",
            _WIPE_SH]


def data_root_wipe(runner) -> bool:
    """Clear `$SIM_DATA_ROOT/runtime`'s CONTENTS as root. True when there was nothing to do or
    the wipe verified empty; False when it failed.

    No-op (True) when the root is unset or not an absolute path. The absolute-path test is the
    belt-and-braces foot-gun guard bash carries: this must never be pointed at `/` or at a
    relative path that resolves somewhere surprising.

    Returns a bool rather than raising so each caller keeps bash's own policy: a bring-up
    preflight fail-louds on False (a leaked datadir shifts the deploy nonces), while
    `tear_down` discards it (`soak_data_root_wipe || true`) because a teardown must not turn a
    passing case into a failing one.
    """
    root = data_root()
    if not root or not root.startswith("/"):
        return True
    return runner.run_ok(wipe_argv(root), note="data-root-wipe")


def ram_root_purge(root: str = None) -> bool:
    """RAM-mode purge-on-fail (bundle-20260717T-hostfreeze). True if it fired.

    When a run FAILS with a `/dev/shm` data root, the tmpfs pages the reth trees occupy are NOT
    released by teardown alone: `down -v` drops the named volume, but the bind target's files
    are host-owned and stay resident until something unlinks them, so a failed RAM run keeps
    holding gigabytes of RAM.

    Deliberately a HOST-side `rm -rf`, NOT the container wipe above: under the very memory
    exhaustion that triggers this, `docker run busybox` may fail to start. The unlink is safe
    while nodes still hold the files open — POSIX reclaims the tmpfs pages when the last fd
    closes (the `down -v` moments later).

    HARD-GUARDED: fires ONLY when the root resolves STRICTLY under `/dev/shm/` with a non-empty
    component past it — never `/dev/shm` itself, never `/`, never a non-tmpfs path, never a
    `..`-traversing one. A disk-backed or unset root is a silent no-op; only the sim's failure
    path calls this, and it is inert everywhere else.
    """
    root = (data_root() if root is None else str(root).rstrip("/"))
    if not root or ".." in root:
        return False
    if not root.startswith("/dev/shm/") or not root[len("/dev/shm/"):]:
        return False
    print(f"ram_root_purge: releasing RAM data root {root} "
          "(post-fail tmpfs purge — host-side rm)", flush=True)
    shutil.rmtree(root, ignore_errors=True)
    return True
