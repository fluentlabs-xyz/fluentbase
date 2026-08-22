## graphify (optional structural search)

- `graphify-fresh query|explain|path|affected …` — OPTIONAL helper for structural
  code navigation (where is a symbol, who calls it, call chains). Its first line
  reports graph freshness — read it. Results are navigation HINTS, not facts:
  relay-provenance rules apply, verify in code before asserting. Pass uniquely-named
  symbols — bare `Voter`/`Actor`/`Config` can land on an unrelated same-named node.
- Architecture, invariants, and "what we already established" live ONLY in
  .claude/dpos_architecture/ (TOC first) — the graph carries no doc layer.
- Rebuild deliberately when you want a current graph: `graphify update . &&
  graphify-prune` (prune is mandatory after update — raw output floods the graph
  with type-position edges). No automatic update obligation exists.
- A stray `graphify-out/` under `crates/*` breaks cargo/docker (dir without
  Cargo.toml inside a workspace glob); `graphify-prune` sweeps them — check this
  first if a build fails resolving the workspace.
