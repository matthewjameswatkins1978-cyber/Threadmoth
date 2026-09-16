# Threadmoth 1.10 context-economy evidence

This document records the first release-gate measurements against the retained
[Threadmoth 1.9.1 baseline](context-economy-baseline-1.9.1.md). Threadmoth
`1.10.0` carries these bounded inspection facilities; release proceeds only
after verified main-branch acceptance and tagging.

## Measured outline exposure

Measurements used the release binary built from this branch and counted UTF-8
bytes in the JSON response. The source file was read and hashed by Threadmoth
for every request; the outline cache never replaced that identity check.

| Fixture | Source bytes | Source lines | Outline JSON bytes | Entries | Available | Truncated |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| Rust | 370 | 17 | 3,428 | 5 | yes | no |
| Python | 287 | 13 | 3,033 | 4 | yes | no |
| TypeScript | 243 | 11 | 3,044 | 4 | yes | no |
| JSON | 81 | 7 | 787 | 0 | no, honest unsupported provider | n/a |
| Markdown | 244 | 15 | 898 | 0 | no, honest unsupported provider | n/a |
| `src/pipeline.rs` | 141,692 | 3,780 | 17,125 | 32 | yes | yes |

For the large-file F1 path, the 1.9.1 baseline broad-read exposure was
141,692 source bytes and 3,780 lines. The bounded outline exposed 17,125 JSON
bytes, then one selected expansion exposed 2,011 JSON bytes containing a
754-byte syntax region. The outline-plus-expansion response payload was
19,136 bytes: an 86.5% reduction against the baseline broad source payload.
This is payload measurement, not a token estimate; the host did not expose a
reliable token counter, so no token saving is claimed.

## Behavioural field matrix

- F1 large source: passed. Outline is capped at 32 in the measurement, marks
  truncation, and one exact handle expands to a bounded region.
- F2 repeated unchanged fact: passed. One long-lived MCP process reports
  `derived` then `cache_hit` while re-reading the current source identity. The
  bounded cache is deterministic least-recently-used: hits move to the back
  and eviction removes the least recently used entry from the front.
- F3 stale handle: passed. A changed source returns a stale observation refusal
  and leaves the changed bytes untouched.
- F4 ambiguous structure: passed. The outline reports multiple exact entries;
  it does not rank, select, or fuzzy-relocate a target.
- F5 unsupported provider: passed. JSON and Markdown return identity plus an
  explicit unavailable outline rather than pretending to have syntax coverage.
- F6 safety and regression matrix: passed by the existing suite plus the new
  inspect/handle/MCP tests. Wrong successful mutations remain zero.

Observation handles are read identities bound to the observation domain,
source SHA-256, normalized path, provider, language, syntax kind and exact
byte range. They are not mutation authorization.
