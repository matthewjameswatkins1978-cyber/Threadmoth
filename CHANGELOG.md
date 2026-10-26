# Changelog

## 1.22.0 - Newline Firewall and Agent-Friendly Editing

### Added

- **Core Newline Firewall:** Automatically preserve uniform LF and CRLF files while allowing safe, locally inferred edits in mixed-newline files. A central pre-commit check applies across supported mutation routes, including prepared plans and transactions.
- **Deliberate Conversion Controls:** Explicit CLI and MCP newline conversion operations and conversion authorisation, separate from exact-match semantics.
- **Simpler Agent Editing:** File-based search/replacement inputs, expected-match counts, replace-all safeguards, and compact multi-file edit documents.
- **Permanent Adversarial Coverage:** Terrorbats CRLF Armageddon regression suite with a bounded deterministic fuzz profile in CI and an opt-in 1,000-case assault.

### Fixed

- Accidental CRLF/LF injection from Windows-generated replacement text, including the Hamesh regression.
- Mixed-file line-identity bugs affecting deletions and reordering, with byte-level preservation validation.
- Diff planner splitting CRLF boundaries, multiline candidate refinement, edit-document effect-budget underestimates, and Unicode near-miss slicing.
- Misleading newline diagnostics and preservation evidence.

### Compatibility

- Wire protocol remains `1.4.0`. Conversion authorisation is additive and defaults to disabled.
- Release version updated from `1.21.0` to `1.22.0`.

## 1.21.0 - Toolbelt Structural Editing and Reliability Consolidation

### Added

- **Toolbelt Attachment Architecture**: Extensible, non-mutating specialist instrument integration while retaining exclusive mutation, verification, and rollback authority within Threadmoth ("Different instruments, one dependable editing contract").
- **Structural AST Pattern Editing (`ast-grep`)**: Added `threadmoth structural --pattern <P> --rewrite <R>` CLI command and `threadmoth_structural` MCP tool.
  - Executes `ast-grep` in proposal streaming mode (`--json=stream`) without granting external tools direct disk-write authority.
  - Translates AST match ranges directly into Threadmoth's guarded transactional pipeline with pre-scan snapshot hash validation, monotonic non-overlapping span verification, and strict workspace containment.
  - Supports `--preview` for non-mutating in-memory inspection and `--plan <FILE>` for decoupled inspection via `threadmoth explain` and application via `threadmoth apply-plan`.
- **Independent Evaluation Lab (`lab/`)**: Standalone black-box test harness using standard Python with 14 acceptance test fixtures, baseline evidence records, and semantic LSP feasibility documentation.
- **Structured Rollback Reporting**: `RollbackReport` provides explicit accounting (`total`, `restored`, `failed`) and evidence preservation across all transaction members on partial or failed rollback.

### Fixed

- **Crash Recovery & Commit Marker**: Introduced explicit two-phase `JournalPhase` (`Prepared` -> `Committed`) synced via `mark_journal_committed` before journal removal. Acknowledged successful transactions are never reversed by later recovery, and uncommitted or failed postcondition states never become commits.
- **TOML Precision & Safety**:
  - `Set` operations preserve existing AST decor (inline comments and whitespace around `=`).
  - `Insert` and `RenameKey` operations refuse with `DestinationExists` if the target key is already occupied.
  - Preserves CRLF line endings on modified TOML files without triggering newline mismatch refusals.
- **CLI Refusal Diagnostics**: Deduplicated structured format recovery hints, removing redundant repeated `capabilities --for <path>`.

### Compatibility

- Wire protocol remains `1.4.0`. All existing exact text, syntax, and structured providers remain fully backward-compatible.

## 1.20.0 - Reliability, Simplicity and Agent-First Editing

### Added

- Multi-file edit documents support `#` comments outside of blocks, trailing whitespace on delimiter markers (`<<<<<<< SEARCH `, `======= `, `>>>>>>> REPLACE `), and preserve single blank line block bodies.
- **Scoped Anchor Disambiguation**: Added `--within <ANCHOR>` to `threadmoth edit` and `threadmoth prepare-text`, and `within: string` to `threadmoth_edit` and `threadmoth_prepare_text` MCP tools. Reliably scopes single-file replacements to a specific function, class, or method declaration in code files (15+ Tree-sitter grammars), section in TOML/INI files (`[section]`), or heading in Markdown documents (`# Heading`).
- Strict fail-closed boundary enforcement: completely forbids speculative indentation-based guessing. Enforces declaration header boundary rules (`anchor_start < body_start`), rejecting body anchors.
- Integrates seamlessly with existing `RegionGuard` contract, validating anchor uniqueness and hash invariants across transaction preparation and application lifecycles.
- SIMD-accelerated line splitting and newline profiling via `memchr` for high-throughput exact editing (~1.8x speedup on large text and long-line operations).
- In-memory prepared-plan transaction certificates faithfully populate `changed_line_ranges` and `bounded_diff`.
- Canonical SIMD `newline_profile` integrated across all engine, pipeline, text provider, source preparation, and metadata inspection surfaces.
- Comprehensive regression coverage verifying exact physical byte coordinates in `changed_ranges`, pre/post-hash invariants, prefix/suffix byte preservation, and certificate metadata truthfulness across LF, CRLF, and mixed sources.

### Fixed

- Resolved Myers diff candidate refinement failure on complex multiline replacements (`RenderMono` reproduction) by safely falling back to exact changed-block replacement when token-level refinement yields non-matching candidate bytes.
- Enhanced refusal diagnostics on candidate reconstruction mismatches, reporting byte length and first mismatch offset for rapid diagnosis.
- Unified newline profile detection with SIMD `memchr2`, ensuring accurate reporting across LF, CRLF, mixed, and empty files without false `line_endings_changed` flags on uniform CRLF.

### Scope Reconciliation & Deferrals

- Fully delivered: one-call unified editing (`threadmoth edit`), edit documents (`.tm`), stdin piping (`edit -`), non-mutating preview (`--plan`), byte fidelity fixes, parser robustness, and "Quick as a Rabbit" SIMD acceleration.
- Deferred: cross-process background daemon caching and speculative concurrency, as baseline cold-start (12–15 ms warm execution) and SIMD exact throughput are already exceptional without the architectural risk or complexity of speculative caching.

### Compatibility

- Core deterministic mutation engine, protocol 1.4.0, exact matching, effect budgets, source guards, and certificates are fully preserved.

## 1.19.0 - Unified editing interface and progressive discovery

### Added

- `threadmoth edit FILE --search OLD --replace NEW` provides a unified editing interface for everyday mutations, applying and verifying unambiguous edits in a single invocation.
- `threadmoth edit FILE.tm` and `threadmoth edit -` execute compact multi-file edit documents and piped changes directly.
- `--plan` on `threadmoth edit` enables explicit non-mutating preview and in-memory review.
- `threadmoth_edit` MCP tool provides one-call editing with strict input-shape validation.
- Progressive recovery discovery surfaces grounded candidate line numbers and context snippets on target ambiguity refusals without guessing.

### Fixed

- Outcome reporting distinguishes `APPLIED`, `NO_CHANGE`, `REFUSED`, and `FAILED`. `NO_CHANGE` results exit 0 and are never reported as `REFUSED`.
- Strict validation rejects incomplete `--search`/`--replace` pairs, contradictory arguments, and malformed MCP requests before filesystem access.
- In-memory plans previewed without an output path explicitly notify callers that no artifact was written to disk.
- Cleaned up stale version references across documentation and agent skills; skills discover version from installed runtime rather than redundant frontmatter.

### Compatibility

- Core deterministic mutation engine, protocol 1.4.0, exact matching, source guards, and certificates are fully preserved.

## 1.18.0 - Source-aware multi-file review

### Added

- `prepare-edits --review` displays the existing bounded source diff during preparation while leaving application as a separate command. Truncated reviews include an explicit full-review command.
- `threadmoth schema <scope>` now returns only the selected request, response, certificate, transaction, or metadata contract; the no-scope form still returns the complete schema bundle.

### Compatibility

- Exact matching, mutation engine, protocol 1.4.0, MCP interfaces, source guards, and certificate formats are unchanged.

## 1.17.0 - Compact exact multi-file edit documents

### Added

- prepare-edits accepts concise multi-file documents with ordered exact SEARCH/REPLACE blocks and prepares one guarded transaction without writing source files.
- Uniform LF or CRLF is derived from each source; mixed-newline files and ambiguous, missing, stale, or unsafe targets fail closed.
- Regression fixtures cover dependent and multi-file edits, 24 edits, escaped delimiters, UTF-8, budgets, path confinement, links, and comparative input size.

### Compatibility

- This adds a CLI authoring format only. Protocol 1.4.0, MCP surfaces, exact operations, prepared plans, certificates, and updater behavior retain their existing contracts.

## 1.16.1 - Safe plan commands across shells

### Fixed

- `prepare-text --summary` emits shell-labeled Review and Apply commands with literal plan-path quoting appropriate to PowerShell on Windows or POSIX shells on Unix.
- Paths that cannot be represented safely and faithfully on one command line receive non-executable guidance rather than misleading commands.
- Added real shell round-trip regression tests for quotes, interpolation, special characters and unrepresentable paths.

### Compatibility

- Patch-level CLI guidance fix only. Exact editing, source preservation, plans, certificates, workspace guards and protocol version 1.4.0 are unchanged.

## 1.16.0 - Agent usability and source-aware input

### Added

- prepare-text PATH --edit-files OLD_FILE NEW_FILE accepts repeated, ordered pairs of bounded workspace-confined UTF-8 files for faithful multiline input through the existing exact preparation pipeline.
- Preparation refusals now return exit code 2, preserve refusal certificates, and offer structured file-input recovery for missing exact targets; line-ending guidance appears only when supported by observed source.
- inspect --json is accepted as a compatibility spelling for its existing JSON output, and budget.allowed_path_prefixes may be omitted with the existing empty-array semantics.
- Orient, help, CLI/MCP documentation and examples explain the concise and file-based input routes.

### Compatibility

- Protocol version remains 1.4.0. Existing CLI inline edits, JSON requests, exact matching, workspace confinement, plans and certificates retain their existing semantics.


## 1.15.0 - Guided source-aware text editing

### Added

- `threadmoth prepare-text PATH --edit OLD NEW --output PLAN --summary` provides a compact CLI path for ordinary exact text edits; repeated `--edit` values prepare ordered same-file changes.
- Preparation summaries report the observed source hash and newline profile, write only the requested guarded plan, and print the exact review and apply commands.
- Plan review identifies the next action; stale plans direct re-preparation. MCP preparation discovery documents previewing the prepared request and applying the unchanged plan.
- `threadmoth examples source-text`, orient, help and agent documentation point to the same workflow.

### Preserved

- Exact matching, newline adaptation rules, refusal certificates, source-read-only preparation, workspace confinement, authority, budgets, stale checks and apply certificates.
- The complete JSON request interface remains available for advanced callers.

### Live agent comparison

- Same isolated Pi 1.1.0 model and prompt on the same CRLF fixture: both tasks succeeded. The 1.14 baseline used 14 tool calls, one budget refusal/retry, 1,277 request JSON bytes and about 13,054 tokens. The 1.15 run used 9 tool calls, no Threadmoth refusal, about 422 bytes of prepare/review/apply command text and about 6,256 tokens.
- The 1.15 agent still made three failed setup probes in the temporary workspace. It read the target once, used no line-ending hex inspection or raw edit fallback, and preserved the comment and all five CRLF line endings. These are single-run observations, not population estimates.

## 1.14.0 - Source-aware text preparation

### Added

- `threadmoth_prepare_text` and `threadmoth prepare-text` accept compact ordered exact text replacements and return the existing portable prepared plan without writing the source.
- Uniform LF/CRLF conventions are derived from the observed file. Mixed or absent conventions require `newline_policy: "exact"` and literal line-ending bytes.
- Ordered replacements resolve against one evolving in-memory source and are committed as one stale-safe, budgeted operation.
- Protocol 1.4.0 adds `text.replace_sequence`; protocols 1.3.1, 1.3.0, 1.2.0 and 1.1.0 remain accepted with their prior semantics.

### Preserved

- Exact text matching remains byte-exact after the explicitly defined newline adaptation; missing and duplicate targets still refuse.
- Preparation is read-only. Applying a returned plan revalidates the observed hash, authority, cardinality, budgets and exact operation before commit.

### Verification

- The source-aware workflow regression harness compares retries, tool calls and request bytes against the prior full-request workflow. See the 1.14.0 release workflow for complete test, lint, Windows build and security-regression evidence.

## 1.13.0 - Agent-native discovery and recovery

### Added

- `threadmoth orient` and `threadmoth orient --json` provide a compact workspace-bound discovery map and preserve the documented exit codes.
- `threadmoth help PROVIDER` and `threadmoth help PROVIDER:OPERATION --json` project provider selectors, operation metadata and matching executable examples.
- MCP discovery accepts canonical `file_path` and `selector` names while retaining the existing `path`, `file`, `for_path` and selector `path` aliases.
- MCP refusal results include a compact structured envelope; workspace-root mismatch recovery offers checkout-local CLI and host-configuration inspection without rebinding authority.
- Suggestion templates are checked against the advertised provider-operation pairs before they are returned.

### Preserved

- The workspace root remains bound to the process configuration. Request paths and MCP roots do not grant or rebind authority.
- The existing `Certificate` refusal and recovery fields, operation wire tags, and exit-code contract remain compatible.
- JSON Pointer remains unsupported; dotted/bracket selector diagnostics continue to reject ambiguity.

### Verification

- See the 1.13.0 release workflow for focused/full test, static-analysis, package and cross-platform security-regression results.

## 1.12.0 - Teachback and provider fidelity

### Added

- Workspace-root mismatch recovery now exposes `suggest` alongside inspection and refusal explanation commands while preserving caller choice and the original refusal.
- `suggest PATH` emits provider-specific JSON, JSONC, TOML, YAML, dotenv, INI, Markdown and syntax-aware code templates; unsupported provider/goal pairs return provider operation choices instead of generic text templates.
- CLI subprocess regressions lock the documented exit codes: 0 for applied/no-change, 2 for refusal and 3 for runtime failure.

### Fixed

- JSON and JSONC capability metadata now advertises the supported dotted/bracket selector syntax instead of claiming JSON Pointer support. Pointer-looking selectors receive an explicit structured correction diagnostic.
- Kept canonical path containment checks and their cross-platform regression suite in the release gate.
## 1.11.0 - Workspace scope and containment

### Added

- Absolute targets outside the process workspace now return a structured, choice-required checkout-local CLI recovery. The recovery preserves the refusal and does not broaden mutation authority.

### Fixed

- Enforced path budgets against resolved filesystem targets and destinations, including plans and transactions.
- Rejected lexical parent traversal before scope evaluation and resolved existing path ancestors before authorizing writes.
- Added Windows junction and symlink containment regressions; canonical path comparisons remain component-boundary and case-sensitive.

### Release

- Published portable and x86-64-v3 Windows, Linux and macOS artifacts with SHA-256 files and a release manifest.

## 1.10.0 - Context Economy

This release adds deterministic bounded structural inspection to the existing
`inspect` CLI/MCP surface. Identity-only inspect remains compatible; supported
syntax targets can expose compact outlines and expand one hash-bound observation
without granting mutation authority. Stale observations, malformed syntax,
unsupported providers and over-budget expansions refuse honestly. The
long-lived MCP adapter uses only bounded non-persistent LRU outline reuse after
rechecking current source identity.

The `v1.10.0` release workflow publishes per-archive SHA-256 files and a release
manifest.

## 1.9.1 - Consistency and hardening

Threadmoth 1.9.1 makes the Target Registry authoritative for shorthand target
resolution and adds explicit string-value ergonomics without weakening strict
JSON value handling.

### Fixed

- Fixed registered INI-family filenames including `setup.cfg`, `tox.ini`,
  `pytest.ini` and `.editorconfig` being rejected by extension-only shorthand
  detection.
- Fixed `.env` filename families being rejected by shorthand provider
  detection.
- Aligned CLI and MCP value-setting resolution through one shared registry
  resolver.

### Added

- Added `set-value --string` and the `set-string` convenience shorthand for
  explicit UTF-8 string values.
- Added regression coverage for special filenames, value typing, refusal
  safety and CLI/MCP parity.

### Safety

- `set-value` remains strict JSON by default and malformed values still refuse;
  invalid JSON is never silently coerced into a string.
- No semantic codemods or plans/proof work were added; those remain future
  scope.

## 1.9.0 - Coverage and performance foundations

Threadmoth 1.9.0 makes the coverage model explicit without expanding into compiler, formatter, package-manager or infrastructure semantics.

### Added

- Canonical target registry for deterministic file classification, aliases, extensions, exact filenames, provider ownership and fallback metadata.
- AI-facing `structured`, `syntax`, `region`, `exact` and `opaque` understanding levels plus preservation metadata and path-scoped discovery.
- Tree-sitter syntax targeting for Java, C#, PHP and HCL / Terraform syntax through the existing parser-locates/Core-mutates boundary.
- Source-preserving nested YAML path targeting for mappings and sequence indexes, with conservative fail-closed handling of advanced constructs.
- Conservative INI-style section/key provider, including registered filenames such as `setup.cfg`, `tox.ini`, `pytest.ini` and `.editorconfig`.
- Explicit portable, `x86-64-v3` modern and local `target-cpu=native` build flavours, build metadata and helper scripts.
- Named `maxperf` Cargo profile for measured release optimization experiments.
- Release/updater flavour selection and modern Windows/Linux artifact wiring.
- Coverage, target-registry and performance documentation plus current local build-flavour evidence.

### Deliberately deferred / bounded

- Kotlin, Swift and GNU Make syntax remain deferred until parser/integration quality clears the admission bar.
- Dockerfile, Makefile and Java `.properties` remain explicit exact-text targets rather than overstated structural support.
- HCL support does not include Terraform resource/provider/state semantics.
- Advanced YAML constructs remain refusal boundaries where local preservation cannot be proved.
- The full compiler optimization matrix and separate PGO training/validation remain follow-up performance work; they are not required to publish the portable and modern 1.9.0 artifacts. They remain prerequisites for declaring a final performance winner or shipping PGO.

The correctness requirement remains unchanged: **wrong successful mutations must remain zero**.

## 1.8.1 - Completion and hardening

Threadmoth 1.8.1 hardens the 1.8 agent workflow without expanding semantic authorship. It adds actionable refusal recovery, bounded cross-process mutation locking, strict structured schema diagnostics, safe shorthand entry points, structured doctor output, and adversarial plan/filesystem coverage.

- Added deterministic refusal remedies and complete guarded retry templates for ambiguity, stale state, missing targets, and effect-budget refusals.
- Added `replace-exact`, `set-value`, and `create-file` CLI shorthands plus equivalent exact-replace/set-value MCP tools through the canonical Core pipeline.
- Added `WORKSPACE_BUSY` fail-closed locking across mutations, plan application, transactions, and recovery.
- Added protocol 1.3.1 while continuing to accept protocol 1.3.0, 1.2.0, and 1.1.0.
- Added `threadmoth doctor --json` and structured schema diagnostics for strict parser failures.
- Added release, plan, path-identity, and refusal-recovery regression coverage.

## 1.8.0 - Plans and proof

Threadmoth 1.8 makes guarded mutations portable and provable. It adds deterministic serialisable plans, exact stale-state rechecking, prospective and committed postcondition checks, and an explicit CLI-only updater for standalone installations.

- Added `threadmoth plan`, `threadmoth apply-plan`, and `threadmoth explain --plan`.
- Added bounded `file_exists`, `file_absent`, `sha256`, and `literal_count` assertions.
- Added protocol 1.3 capability flags, plan limits, and MCP plan/apply-plan parity.
- Added official GitHub-release self-update with archive SHA-256, GitHub digest, extracted-binary version, and safe replacement verification.
- Preserved local-only mutation and existing preview, mutate, transaction, recovery, and candidate-guard flows.

## 1.7.1 - Repository and distribution hardening

Threadmoth 1.7.1 keeps the 1.7 agent-usability runtime stable while making the public repository and release process consistent with the Threadmoth identity.

- Renamed the canonical GitHub repository to Threadmoth and updated links and package metadata.
- Added the native Antigravity adapter manifest and current integration documentation.
- Preserved refusal-first mutation semantics and published reproducible release artifacts with checksums and a manifest.

## 1.7.0 - Agent usability

Threadmoth 1.7.0 makes the refusal-first boundary easier for agents to discover, inspect, and recover through.

### Added

- MCP parity for inspect, suggest, explain, path-scoped capabilities, and non-writing transaction preview.
- Protocol 1.2 candidate selection guards with deterministic physical selection IDs bound to the exact observed file identity.
- Provider-preserving refusal recovery and adversarial text/code candidate-selection coverage.
- macOS Apple Silicon and Intel release targets.
- A short agent integration loop and copy-paste MCP configuration.

### Changed

- Protocol 1.1 requests and recovery journals remain accepted for compatibility.
- Common `.env.*`, Dockerfile, Makefile, Cargo, package, TypeScript, and config filename detection is deterministic without content guessing.

## 1.6.0 - Hardening release

Threadmoth 1.6.0 strengthens Threadmoth against several newly identified ambiguity, recovery, protocol, and pathological-input edge cases.

### Fixed

- False AST ambiguity caused by duplicate Tree-sitter spans.
- Recovery journal byte-array expansion.
- Pathological long-line diff refinement cost.
- JSON-RPC notification responses from the MCP stdio server.

### Added

- MCP `threadmoth_preview`.
- Regression and adversarial coverage for same-span AST nodes, binary-safe recovery, long-line refinement, and MCP protocol behavior.
- A dedicated long-line refinement benchmark case.

### Changed

- Recovery journal payloads use compact base64 strings and retain v1.5.1 decimal-array read compatibility.
- Diff planning trims common prefix/suffix bytes and applies a bounded byte-refinement cutoff.

This release preserves Threadmoth's source-preserving, refusal-first architecture: providers plan byte ranges, while Core remains responsible for guards, atomic writes, verification, and certification.
