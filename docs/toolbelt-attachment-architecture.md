# Design Decision: Threadmoth Toolbelt Attachment Architecture

## 1. Context & Objective

Threadmoth is a deterministic, source-preserving mutation authority for AI coding agents. While Threadmoth excels at bounded, exact-text, structural section, and configuration edits, agents frequently face cross-file structural transformations (e.g., transforming AST patterns across dozens of files, rewriting function calls with metavariables, or migrating API usages).

The goal of the **Threadmoth Toolbelt** is to evolve Threadmoth into an extensible editing workbench without creating another agent, IDE, or heavyweight plugin framework.

**Guiding principle: Different instruments, one dependable editing contract.**

## 2. Baseline Audit: Existing Threadmoth Capabilities

Threadmoth already provides a comprehensive mutation foundation:
- **Exact & Syntax Providers:** Text, JSON, JSONC, TOML, YAML, INI, Markdown, Dotenv, Patch, and Code/Web (tree-sitter based).
- **Inspection & Discovery:** `threadmoth capabilities`, `threadmoth suggest`, `threadmoth orient`, `threadmoth explain`.
- **Guarded Planning & Desired-State:** `prepare-text`, `apply-plan`, `DesiredStateOperation::ReplaceEntire`, byte-range edits.
- **Transactional Safety & Recovery:** Two-phase commit journals (`Prepared` -> `Committed`), exhaustive reverse-order rollback, crash recovery.
- **File Lifecycle:** Guarded create, delete, rename, move with stale-file checks and path containment.
- **Agent Integration:** Standardized JSON certificates, effect budgets, schema validation, and MCP tools.

### Identified Agent Pain Points
1. **Repeated AST Patterns:** Transforming repetitive structural patterns across multiple files requires agents to manually locate nodes, write individual text edits, or run unverified scripts.
2. **Safety of External Tools:** External CLI tools (such as `ast-grep`, `comby`, or `jscodeshift`) when run directly by agents can mutate working trees without Threadmoth's guardrails, pre-hash verification, effect budgets, or rollback capability.

## 3. Attachment Architecture: The Minimal Contract

Attachments have two primary responsibilities:

1. **Discover:** Advertise what the attachment can do, supported languages/file extensions, prerequisites (e.g. installed CLI binaries), and limitations.
2. **Propose:** Produce a deterministic stream of proposed changes against observed source snapshots without modifying live files.

### Host Boundary & Safety
The Threadmoth host retains exclusive mutation authority:
```
External Specialist Tool (e.g., ast-grep)
         │ [non-mutating proposal JSON]
         ▼
Threadmoth Attachment Adapter
         │ [validates offsets, containment, overlaps, hashes]
         ▼
Threadmoth Guarded Pipeline (prepare-plan / desired-state)
         │ [pre-hash, budget, stale checks]
         ▼
Atomic Execution & Two-Phase Recovery
         │ [candidate write, postcondition check, commit marker]
         ▼
Machine-Readable Certificate (Diff, Hashes, Outcome)
```

## 4. First Attachment: Structural Editing with `ast-grep`

### Integration Approach
- **Engine:** Standalone `ast-grep` executable (`ast-grep run --pattern <P> --rewrite <R> --lang <L> --json=stream`).
- **Reuse:** Leverage ast-grep's mature, permissively licensed (MIT) AST pattern matching and rewrite syntax (`$METAVAR`, `$$$METAVARS`).
- **Non-mutating Extraction:** ast-grep runs in proposal mode only (never `--update-all` or `-i`).
- **Direct Offset Translation:** Parse ast-grep streaming JSON records:
  - `range.byteOffset.start` .. `range.byteOffset.end`
  - `replacement` text
  - `file` path
- **Overlap & Conflict Detection:** Validate that matches within the same file are non-overlapping and strictly monotonic in byte offsets.
- **Workspace Containment:** Normalize all file paths, ensure they reside strictly within the workspace root, and reject symlink traversal outside the workspace.
- **Security:** When `ast-grep` >= 0.50, pass `--custom-languages ignore` to prevent untrusted native parser execution.
- **Line Ending & Multibyte Safety:** Byte offsets are checked against the observed UTF-8 byte stream. Line endings (LF/CRLF) are preserved across unchanged regions.

## 5. Deliberately Deferred Scope

- **Semantic Refactoring (LSP):** Language-server symbol renaming, type-aware operations, and cross-file reference updates will form a distinct future Semantic attachment reusing the LSP `WorkspaceEdit` standard.
- **Workspace Lifecycle Mutations:** Multi-file moves coordinated with import updates.
- **Plugin Daemons / Dynamic Shared Objects:** No IPC daemons, background servers, or dynamic C-ABI plugin loading. All attachments use discoverable child process execution or native Rust modules.
- **LineDoctor:** Coordinate with LineDoctor's existing CLI/interface separately without duplicating its functionality.
