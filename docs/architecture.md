# Architecture

Threadmoth keeps one mutation authority:

```text
OBSERVE -> IDENTIFY -> GUARD -> PLAN -> VERIFY PROSPECTIVE -> MUTATE -> VERIFY COMMITTED -> CERTIFY
```

All CLI, MCP, shorthand, plan and transaction paths converge on Core preparation, guarding, verification and certification. Cooperating writers take the workspace mutation lock for the full read/prepare/commit/verify boundary; read-only discovery remains unlocked. The lock protects against another cooperating Threadmoth process, not an unrelated editor, so stale hashes and post-commit readback remain part of the guarantee.

> The parser gets to point at the cloth. It doesn't get to re-weave it.

Core observes bytes through `Workspace`, checks protocol compatibility and optional pre-hash/candidate guards, asks exactly one selected provider for a byte-edit plan, applies edits in memory, validates the candidate and owns persistence. Providers understand file structure and propose byte ranges; only Core commits.

## Coverage and target discovery

`src/target_registry.rs` is the canonical user-facing discovery registry. It owns target IDs, aliases, extensions, exact filenames, provider ownership, understanding level, preservation level and explicit fallback availability.

Detection is deterministic and evidence-based. Exact filenames and registered extensions are preferred; recognized `.env` families and shebangs provide narrower evidence; content sniffing can report ambiguity but cannot silently select a provider. Valid unknown UTF-8 falls to the explicit exact-text level. Invalid UTF-8 or binary-like NUL content becomes opaque.

The Tree-sitter grammar registry is an implementation detail of syntax targeting and is keyed to the same canonical target IDs. It is not a second user-facing capability list.

## Providers

Structured providers cover JSON/JSONC, TOML, nested conservative YAML, INI-style configuration and dotenv. Markdown uses a bounded-region model. Code and Web share the Tree-sitter source-node engine. Text, Pattern, strict Patch and Desired State provide explicit non-structural routes. Filesystem lifecycle operations remain separately guarded.

No provider silently falls back to a different provider. A refusal may advertise an explicit weaker route, but the caller must choose it.

## Prepared plans

`threadmoth plan` serialises the existing guarded preparation result without writing. `threadmoth apply-plan` treats that file as staleable and tamperable input: it checks deterministic plan identity, workspace containment, stored pre-images, provider resolution, exact byte edits, effect budgets and assertions before entering the journaled commit path. It then reads committed bytes back and evaluates assertions again.

Preview and ordinary mutation continue to use their existing surfaces; plans are a prepare/commit handoff, not a second transaction engine.

## Two plan constructors

Structural providers locate exact source ranges from observed bytes. The desired-state Diff Planner accepts observed bytes plus explicitly supplied desired bytes, then derives deterministic, bounded, disjoint byte edits. Neither provider nor planner writes files.

Before commit, Core proves that the derived edits produce the exact desired bytes. After commit, it reads the landed bytes and proves the post-hash equals the desired hash. Desired-state mode accounts for all supplied divergence; it does not claim unrelated-byte preservation when the desired state intentionally reformats a file.

## Syntax targeting

AST-grounded targeting means exact text plus a Tree-sitter node boundary plus cardinality. AST-typed targeting adds an explicitly requested `node_kind`. Threadmoth never infers a grammatical role during committed mutation and never unparses or pretty-prints an AST.

## Maintenance boundary

The updater is a separate CLI maintenance boundary. Mutation Core, providers and MCP mutation tools have no updater call path and no network capability. Only an explicit `threadmoth update` invocation contacts the official release repository.

The updater preserves release-flavour selection where supported and still requires the release manifest, the matching archive/checksum asset and executable-version verification before replacement. Package-managed installations are reported rather than overwritten.

The workspace rejects absolute paths, `..` escapes and symlink paths resolving outside the declared root. Commit uses destination-directory staged atomic replacement and reports metadata limits explicitly.

## Process Model: One Long-Lived Connection, Many Verified Operations

For agent harnesses and IDE integrations, Threadmoth defines and follows the approved architectural principle:
**One long-lived connection, many verified operations.**

### Rationale & Lifecycle
1. **Eliminating Per-Operation Process Tax:**
   Spawning a fresh operating system process for each edit incurs image loading, dynamic library resolution, address space allocation, memory mapping, and runtime initialization costs. On Windows, process launch latency alone averages ~13–16 ms for a native binary, and can exceed ~300 ms to ~2,000 ms when invoked through an un-warmed shell environment or PowerShell profile.
2. **Persistent stdio MCP Transport:**
   Agents establish a single long-lived connection (`threadmoth mcp`) bound to the workspace root at session initialization. Subsequent discovery, inspection, text preparation, preview, application, and verification operations flow over standard input/output using JSON-RPC 2.0 messages.
3. **Independent Verification on Every Turn:**
   Connection persistence **never weakens mutation authority or epistemic verification**:
   - Each operation independently reads the current on-disk state.
   - Cryptographic hashes (`pre_hash`, `candidate_hash`, `post_hash`) are computed and checked on every operation.
   - Stale-state protection, cardinality constraints, effect budgets, and path containment are strictly enforced per turn.
   - The persistent observation cache holds parsed AST outline summaries for read acceleration, but never confers authority to mutate.
4. **Crash Safety & Fail-Closed Transport Invariants:**
   - If the worker process terminates, crashes, or times out during a non-mutating operation (such as `threadmoth_prepare_text`), the connection can be safely re-established.
   - If worker termination or timeout occurs during the mutation boundary (`threadmoth_apply_plan`), the client **must never silently retry** the operation. Because the mutation may have partially or completely committed to disk, silent retries could corrupt state or trigger conflicting edits. The harness must report an explicit error and require human or agent inspection of disk state.

