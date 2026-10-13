# Threadmoth extension

Threadmoth is an optional local executable used for precise, source-preserving workspace mutation. The canonical command is `threadmoth`; it must be installed on the user's PATH separately from this extension.

When a requested change is a bounded mutation of a file, or a guarded create/delete/rename/move operation, activate the bundled `threadmoth` skill. Discover the installed version and path-scoped capabilities before choosing a provider:

```text
threadmoth --version
threadmoth capabilities --for PATH --json
threadmoth suggest PATH
```

Threadmoth 1.15.0 provides a compact exact-text route: `threadmoth prepare-text PATH --edit OLD NEW --output PLAN --summary`, then review with `threadmoth explain --plan PLAN --format diff` and apply the reviewed plan with `threadmoth apply-plan --plan PLAN --summary`. It derives uniform LF/CRLF, refuses unresolved targets, and preserves stale checks, authority and certificates. Repeat `--edit` for ordered same-file changes; use `--request` for the full JSON form. Mixed newline files require explicit exact line endings. The inspect surface still provides bounded outlines and stale-safe expansion; treat `REFUSED` as a deliberate safety result and preserve the returned certificate.

Do not silently downgrade a refused structured/syntax operation to a broader raw edit. Do not use Threadmoth for Git, builds, tests, formatter execution, package-manager semantics, Terraform semantics or general network work. The explicit CLI-only `threadmoth update` command is the maintenance exception.
