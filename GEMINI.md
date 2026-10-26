# Threadmoth extension

Threadmoth is an optional local executable used for precise, source-preserving workspace mutation. The canonical command is `threadmoth`; it must be installed on the user's PATH separately from this extension.

When a requested change is a bounded mutation of a file, or a guarded create/delete/rename/move operation, activate the bundled `threadmoth` skill. Always discover installed runtime capabilities rather than assuming a fixed version:

```text
threadmoth --version
threadmoth orient
threadmoth capabilities --for PATH --json
threadmoth suggest PATH
```

For everyday single-file mutations, use `threadmoth edit PATH --search OLD --replace NEW` (or MCP `threadmoth_edit`) to verify, apply, and certify changes in one call. Use multi-file edit documents (`threadmoth edit changes.tm` or stdin `threadmoth edit -`) for coordinated changes, and add `--plan` for a non-mutating preview. The two-step route remains available: prepare with `threadmoth prepare-text PATH --edit OLD NEW --output PLAN --summary` (or `threadmoth prepare-edits`), inspect the diff with `threadmoth explain --plan PLAN --format diff`, and apply with `threadmoth apply-plan --plan PLAN --summary`.

For cross-file AST transformations, pattern-based rewrites, or API migrations, use `threadmoth structural --pattern PATTERN --rewrite REWRITE [PATHS]...` (or MCP `threadmoth_structural`). Note the precise safety guarantee: Threadmoth runs the external engine (`ast-grep`) in proposal streaming mode, enforces pre-scan hashes, monotonic non-overlapping ranges, and strict workspace containment, and retains exclusive disk-write and rollback authority through its guarded transaction pipeline. Threadmoth controls whether proposed changes are applied; this mutation gate does not constitute operating-system sandboxing of the child process itself.

All routes derive uniform LF/CRLF, reject ambiguous or missing targets, and preserve stale checks, effect budgets, and certificates. Mixed newline files require explicit exact line endings. The inspect surface provides bounded outlines and stale-safe expansion (`threadmoth inspect`). Treat `REFUSED` as a deliberate safety result, and do not silently downgrade a refused structured, syntax, or structural operation to a broader unverified edit.

Do not use Threadmoth for Git, builds, tests, formatter execution, package-manager semantics, Terraform semantics or general network work. The explicit CLI-only `threadmoth update` command is the maintenance exception.
