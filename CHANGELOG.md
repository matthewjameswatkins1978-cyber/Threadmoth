# Changelog

## 1.10.0 - Context Economy release candidate

This unreleased candidate adds deterministic bounded structural inspection to
the existing `inspect` CLI/MCP surface. Identity-only inspect remains
compatible; supported syntax targets can expose compact outlines and expand one
hash-bound observation without granting mutation authority. Stale observations,
malformed syntax, unsupported providers and over-budget expansions refuse
honestly. The long-lived MCP adapter uses only bounded non-persistent LRU
outline reuse after rechecking current source identity.

The candidate is tracked in draft PR #51. It is not merged, tagged or
published.

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
