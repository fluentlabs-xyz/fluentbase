"""exit_codes.py — the process return codes every case answers with.

They live in `core/` rather than in `cli.py` because a case module may not import the entrypoint:
`cli` reaches down into `cases` (`_stack_golden`), so the reverse edge is an import cycle, and
`tests/test_layering.py::test_no_import_cycles` fails on it. `cli` re-exports the names, so the
aggregator reads them from the same definition the cases return.

The distinction that motivates the set is FAIL vs ERROR. Both used to be 1, so a broken bring-up
and a false verdict were the same number to anything reading the exit status — and a suite that
cannot tell "the property is false" from "the run never got far enough to look" reports
infrastructure noise as a red property.
"""

from __future__ import annotations

RC_PASS = 0
RC_FAIL = 1          # a verdict came back false
RC_USAGE = 2         # unknown argument / bare group
RC_ERROR = 3         # infrastructure: bring-up failure, ProcError, uncaught exception
RC_INCONCLUSIVE = 4  # not enough data to judge
