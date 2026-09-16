# Threadmoth 1.9.1 context-economy baseline

Baseline captured before the 1.10 implementation on starting SHA
`2dcf3054a47d808db46abfabf914cd629da7a3a7`.

## Method

The 1.9.1 `inspect` command is identity-only. A representative agent task that
needs source structure therefore requires a broad source read, followed by one
identity inspection. Repeating the same deterministic question in one session
requires another inspection; the 1.9.1 CLI/MCP path provides no observation
handle or process-local reuse evidence. Token counts were not available from
the local host and are intentionally not estimated.

The source-byte and logical-line counts below were measured from the fixed
fixtures in `tests/fixtures/context_economy/` plus the large existing
`src/pipeline.rs` file. `inspect_output_bytes` is the UTF-8 byte count of the
pretty JSON identity response; `source_bytes` is the source exposure required
by the broad-read workflow.

| Fixture | Source bytes | Logical lines | 1.9.1 inspect output bytes | Broad reads | Repeated unchanged inspections | Reuse hits |
|---|---:|---:|---:|---:|---:|---:|
| `rust_sample.rs` | 370 | 17 | 790 | 1 | 2 | 0 |
| `python_sample.py` | 287 | 13 | 794 | 1 | 2 | 0 |
| `typescript_sample.ts` | 243 | 11 | 802 | 1 | 2 | 0 |
| `config.json` | 81 | 7 | 682 | 1 | 2 | 0 |
| `document.md` | 244 | 15 | 793 | 1 | 2 | 0 |
| `src/pipeline.rs` (large task) | 141,692 | 3,780 | 765 | 1 | 2 | 0 |

## Safety baseline

- Starting product version: `1.9.1`.
- `cargo test --locked`: 95 library tests plus all integration/regression suites passed.
- Existing refusal, stale-plan, transaction, MCP, and stress regressions passed.
- Wrong successful mutations: 0 in the existing safety/field evidence.
- The baseline exposes no outline, expansion, observation handle, or cache; these are the 1.10 comparison surfaces.

This document is a retained comparison point. A lower-context 1.10 result only
counts as an improvement when it preserves the same safety and task correctness.
