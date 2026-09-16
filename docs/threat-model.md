# Threat model

Threadmoth 1.10.0 keeps parsers deliberately subordinate: they point at source bytes and validate candidates; they do not regenerate or reformat source.

Threadmoth assumes the caller may have stale context and the target file may be concurrently modified. An optional expected hash rejects stale observations; Core also rechecks identity before commit and reads the committed file back afterwards.

Path traversal, absolute paths and symlink escapes are refused. Ancestor checks are repeated while resolving and commit uses a canonical destination path. This narrows pathname races; no portable userspace API makes replacement immune to an attacker with equivalent filesystem authority.

## Structured and syntax input

Structured providers parse before mutation and validate the prospective candidate where their contract requires it. Malformed or unsupported JSON/JSONC, TOML, YAML, INI and syntax input is refused rather than repaired by guessing.

YAML intentionally fails closed on advanced constructs when local source preservation cannot be proved. A parser successfully recognizing a file is not permission to perform a lossy rewrite.

Unknown valid UTF-8 can use explicit exact routes. Invalid UTF-8 and binary-like NUL content are classified as opaque rather than silently treated as text.

Request, file, plan, assertion, pattern and diagnostic evidence sizes are bounded by advertised resource limits.

## Concurrency and evidence

Cooperating Threadmoth writers share a bounded workspace lock. Unrelated processes can still change files, so stale-state checks and landed-byte verification remain required.

Certificates do not include full file contents. Duplicate diagnostics contain bounded candidate context and hashes. Diff output is bounded; callers should still treat operation values as potentially sensitive.

Atomic replacement protects readers from observing a partially written staged file after the staged file is flushed. Replacement may change timestamps and does not assert ACL/xattr preservation; permissions remain platform-dependent.

## Recovery journals

Recovery journals are validated for structure, supported version, safe transaction ID, workspace-contained member paths, duplicate paths, size limits and SHA-256 consistency before recovery writes. The writer applies the same compact-journal limit before transaction commit. Legacy journal encodings remain readable only where compatibility code explicitly supports them.

The location of a recovery journal is not authenticated provenance. Another same-user process with equivalent filesystem authority may plant or tamper with a journal. Recovery refuses when member state is not provably original or candidate.

## Prepared plans

Prepared plans are untrusted files. They may be stale, edited, oversized, copied from another workspace or path-manipulated. Applying one never bypasses Core guards: schema and deterministic identity are checked, paths remain contained, symlink escapes refuse, every pre-image hash is re-read, exact edits and budgets are revalidated, and prospective plus committed assertions are evaluated. No fuzzy relocation or trusted-plan flag exists.

## Updater and build flavours

The explicit self-updater is the only normal network-capable Threadmoth command. It uses the canonical GitHub Releases repository, accepts stable semantic versions, requires the release manifest plus the matching platform/flavour archive and `.sha256` asset, verifies the selected artifact, and checks the extracted executable version before replacement. Package-managed installations are not overwritten.

Portable and modern x86-64-v3 artifacts are distinct. A portable installation must not be silently upgraded to a CPU-specific binary. Native builds are local-only and are not universal release artifacts.

Checksums provide integrity against accidental/corrupt asset substitution inside the expected release flow; they are not an independent publisher-authentication system. Signature verification remains future hardening unless added by a later release.

## Out of scope

Threadmoth does not defend against an attacker with equivalent account/filesystem authority who can replace the executable, rewrite the repository and modify the same files at will. It also does not provide compiler/type correctness, package-manager semantics, Terraform semantics, formatter semantics or arbitrary command sandboxing.
