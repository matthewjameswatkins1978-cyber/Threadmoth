# Threadmoth extension

Threadmoth is an optional local executable used for precise, source-preserving workspace mutation. The canonical command is `threadmoth`; it must be installed on the user's PATH separately from this extension.

When a requested change is a bounded mutation of a file, or a guarded create/delete/rename/move operation, activate the bundled `threadmoth` skill. Discover the installed version and path-scoped capabilities before choosing a provider:

```text
threadmoth --version
threadmoth capabilities --for PATH --json
threadmoth suggest PATH
```

Threadmoth 1.10.0 can classify files as structured, syntax-aware, bounded regions, exact text or opaque. For large syntax-aware files, the existing inspect surface can provide a deterministic bounded outline and stale-safe bounded expansion. Prefer the strongest advertised route, preview uncertain edits, treat `REFUSED` as a deliberate safety result, and preserve the returned certificate.

Do not silently downgrade a refused structured/syntax operation to a broader raw edit. Do not use Threadmoth for Git, builds, tests, formatter execution, package-manager semantics, Terraform semantics or general network work. The explicit CLI-only `threadmoth update` command is the maintenance exception.
