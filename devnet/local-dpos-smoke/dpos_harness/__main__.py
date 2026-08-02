"""Package entrypoint: `python -m dpos_harness <group> <command> ...` → cli.main."""

from __future__ import annotations

import sys

from .cli import main

if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
