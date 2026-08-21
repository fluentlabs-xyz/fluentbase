# Fluentbase & rWasm Security Audit History

This folder is the durable record of security audits performed against the
**Fluentbase** execution stack (`fluentlabs-xyz/fluentbase`) and its **rWasm**
dependency (`fluentlabs-xyz/rwasm`, plus `revm-rwasm` / `reth-core-rwasm`).

Each internal audit that was tracked in Linear and used to drive fixes has its
own self-contained Markdown report so a future reviewer can re-check the same
surfaces without re-deriving the history from tickets. External vendor reports
are kept as PDFs.

Reports for audits of the **rWasm repository itself** are maintained alongside
that repository, not here — this folder keeps only the Fluentbase host audits
(including the combined pass that reviewed rWasm as a dependency). See
[rWasm reports maintained elsewhere](#rwasm-reports-maintained-elsewhere).

## Conventions

- **File name:** `YYYY-MM-DD-<repo>-audit.md`, where `<repo>` is `fluentbase` or
  `rwasm` and the date is the audit date.
- **Document format (every report):** H1 title `# <Repo> Security Audit — <date>`
  → metadata block (**Date**, **Repository**, **Audited commit**, **Linear**,
  **Fix PRs**, **Focus**) → `## Scope` → `## Result` (severity counts) →
  `## Findings` (grouped by `### Critical/High/Medium/Low`, each finding a
  `#### <ID> — <title>` with a `Severity · Status · Linear · Fix` lead line and
  `Where` / `Impact` / `Remediation` bullets) → optional `## Notes` →
  `## Re-review checklist for future audits`.

## External vendor reports

| File | Vendor | Date |
| --- | --- | --- |
| [`Veridise_2025_09_22.pdf`](Veridise_2025_09_22.pdf) | Veridise | 2025-09-22 |
| [`Cantina_2026_02_23.pdf`](Cantina_2026_02_23.pdf) | Cantina | 2026-02-23 |

## Fluentbase audit reports

Newest first.

| Report | Date | Repo(s) | Crit | High | Med | Low/Info | Fix PRs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| [2026-08-20](2026-08-20-fluentbase-audit.md) | 2026-08-20 | fluentbase | 0 | 1 | 3 | — | #499, #500, #502 |
| [2026-08-05](2026-08-05-fluentbase-audit.md) | 2026-08-05 | fluentbase | 0 | 6 | 28 | — | many (per finding) |
| [2026-07-10](2026-07-10-fluentbase-audit.md) | 2026-07-10 | fluentbase (+ rwasm dep) | 1 | 2 | 4 | 8 | #457–#467, #162–#164 |
| [2026-06-16](2026-06-16-fluentbase-audit.md) | 2026-06-16 | fluentbase | 0 | 2 | 3 | several | #440 |

The 2026-07-10 report is a combined pass: its Fluentbase host findings
(`H-01`, `M-0x`, `L-0x`, `I-0x`) are why it lives here, and it also records the
rWasm dependency findings (`RWASM-01…06`) produced in the same pass.

## rWasm reports maintained elsewhere

Audits whose primary target was the `fluentlabs-xyz/rwasm` repository are kept
with that repository, under the same naming/format convention. For reference:

| Report | Date | Crit | High | Med | Low/Info | Fix PRs |
| --- | --- | --- | --- | --- | --- | --- |
| `2026-08-07-rwasm-audit.md` | 2026-08-07 | 2 | 6 | 7 | — | rwasm #171–#177 |
| `2026-06-18-rwasm-audit.md` | 2026-06-18 | 0 | 0 | 1 | many | rwasm #154 |
| `2026-05-01-rwasm-audit.md` | 2026-05-01 | — | — | — | — | rwasm #153 (+ FLU-32/33) |

> **Also out of scope here:** the Solidity bridge/gateway contract audit of
> 2026-06-18 (Linear `FLU-857`, repo `fluentlabs-xyz/solidity-contracts`,
> PR #103) targets a different repository. Internal crate-level intake audits
> `FLU-30` (`crates/sdk-derive`) and `FLU-31` (`crates/build`) produced no
> tracked Medium+ findings and are not reproduced as standalone reports.

## Provenance

Every report is reconstructed from the corresponding Linear issue and its
sub-issues; the parent Linear IDs are cited at the top of each report so the
original tracking, comments, and PR links remain reachable.
