## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Scope: the graph covers `crates/dpos`, `crates/node`, `devnet` **plus the pinned
commonware checkout** (the 11 crates we depend on) — so it answers "how does our
code sit on commonware" directly. The reth fork is a **separate** graph at
`~/Work/graphs/reth/graphify-out/`; query it with `cd ~/Work/graphs/reth` or
`--graph ~/Work/graphs/reth/graphify-out/graph.json`.

The four verified reference docs (`.claude/DPOS_ARCHITECTURE.md`,
`DPOS_AUDIT.md`, `COMMONWARE_INTERNALS.md`, `RETH_INTERNALS.md`) are extracted
into the graph as **463 concept/rationale nodes** (310 / 47 / 42 / 64) wired to
each other by 516 edges and to the code by 76 — so `explain` on a symbol returns
both its code neighbours and what we already established about it (invariants,
Rule SA/PIN, fork-delta traps, and the audit row that bears on it).
`DPOS_ARCHITECTURE_CHANGELOG.md` is deliberately **NOT** extracted: it is landing
history, not spec, and its retired mechanisms would read as current facts.

Those nodes survive `graphify update` — it merges into the existing graph.json via
`build_merge`, which replaces per `source_file` and only prunes genuinely deleted
files. They are LLM-extracted, so do NOT rebuild the graph from scratch without
re-running the doc extraction — that is what loses them. Editing a `.claude/*.md`
does not refresh its concepts either (`graphify update` is code-only, and
`.claude/` is outside the corpus scan entirely); re-extract that doc deliberately
when it changes materially. Last doc re-extraction: 2026-08-10.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- `graphify affected "<symbol>"` is the reverse traversal — use it to find what breaks if a symbol changes.
- Pass a **uniquely-named** symbol to `explain`/`path`. Name matching is weak: bare
  `Voter`/`Actor`/`Config` can land on an unrelated same-named node (a bash helper,
  a test fixture). If the output warns about an ambiguous match, it picked the wrong
  node — retry with a more specific name.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update . && graphify-prune` to keep the graph
  current (AST-only, no API cost). **The prune step is not optional**: `update`
  rewrites graph.json with raw AST output, which re-floods it with type-position
  `references` edges and single-letter generic nodes (`D`, `R`, `B`). Those turn
  generics into top hubs, bury real symbols in `query`, and route `path` through
  meaningless shortcuts. `graphify-prune` (in ~/.local/bin) strips them and keeps
  the raw graph at graph.raw.json. It is a no-op on an already-pruned graph, so
  running it twice is safe. Same applies to the reth graph.
- `graphify-prune` also sweeps stray `graphify-out/` cache dirs out of the repo.
  graphify creates one at every scan root, and a stray under `crates/dpos/*`
  breaks `cargo metadata` and every docker build (a dir with no Cargo.toml inside
  a workspace glob). It removes cache-only dirs and warns instead of deleting
  anything holding real output. If a build ever fails resolving the workspace,
  check for a stray graphify-out first.
