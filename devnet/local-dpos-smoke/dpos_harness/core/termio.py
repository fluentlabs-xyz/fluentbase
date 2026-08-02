"""termio.py — terminal / stream IO helpers shared by every entrypoint.

Home of `_enable_line_buffering`, which used to live in `cli.py`. `shadow.py` needed it and
reached UP into `cli` to get it — the package's only import edge into the entrypoint module.
Nothing about the behaviour changed; only where it lives.
"""

from __future__ import annotations

import sys


def enable_line_buffering():
    """Root fix for the block-buffered-log defect: when stdout is redirected to a file/pipe
    (the operator's `… >sim.log`), Python block-buffers by default and the log stays 0 bytes for
    minutes — blinding the operator's monitor AND the sender's marker gate. Flip stdout/stderr to
    LINE buffering at every package entrypoint so each newline flushes. Belt: the emission seams
    (EventLog/sender) also pass flush=True."""
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(line_buffering=True)
        except Exception:  # noqa: BLE001 — a non-reconfigurable stream (pytest capture) just keeps its buffering
            pass


def clear_screen():
    """Clear the terminal for `status --watch`'s repaint. Was `os.system("clear")` — a shell plus
    a coreutils process per repaint, and the ONE exec site in the package that bypassed both the
    proc seam and the unit suite's `subprocess.run` guard. `clear` writes exactly these two escapes
    (cursor home + erase-below); emitting them directly needs no process at all, so this is a seam
    exception removed rather than granted. No-ops harmlessly into a redirected stdout."""
    print("\033[H\033[J", end="")
