---
name: threadmoth
description: Use Threadmoth when an AI coding task needs a precise, bounded mutation of JSON, JSONC, TOML, YAML, INI, Markdown, dotenv, pattern, source-code/web syntax, patch, desired-state, or exact text files; guarded file lifecycle operations such as create/delete/rename/move; preservation of unrelated bytes; named sections or syntax nodes; repeated/ambiguous match handling; stale-file guards; effect budgets; or a machine-readable certificate. Prefer it when an unconstrained write could cause collateral edits. Do not use it for Git, builds, tests, formatter execution, package-manager semantics, Terraform semantics, bulk generation, or unsupported opaque file shapes.
license: MIT
compatibility: >-
  Requires the Threadmoth executable on PATH (canonical command: threadmoth).
  Works with any agent that can read files and run local commands.
metadata:
  author: matthewjameswatkins1978-cyber
  version: "1.9.1"
---

# Threadmoth

Threadmoth is a narrow mutation boundary for workspace files. It observes the file or guarded lifecycle target, identifies the intended effect, guards the request, prepares an exact candidate, verifies prospective and committed state, and returns a certificate.

## Canonical ambiguity rule

**Preserve ambiguity when shaping mutation requests. Do not add identifying information that was not supplied by the user or established by evidence.**

**Narrow from evidence, never from imagination.**

An agent may narrow a target only when the specificity comes from an explicit user instruction, a uniquely established observed target, a deliberate caller or user candidate selection, or other trustworthy task evidence that actually resolves the ambiguity. If multiple plausible targets remain, preserve that unresolved choice: inspect or suggest candidates, ask the user to choose, or stop with a clear refusal. Never select the first candidate, the most plausible name, or any target that merely makes progress possible.

It is not a general shell, formatter, compiler, test runner, Git client, package manager, Terraform engine or network tool. The explicit `threadmoth update` maintenance command is a separate CLI-only exception.

## Decide whether to use it

Use Threadmoth when the task changes an existing file through a discovered structured/syntax/region/exact route, or performs a guarded create/delete/rename/move operation on a workspace path and exact scope matters.

It is especially useful when:

- a named section, structured path, syntax node or exact text occurrence is the target;
- a file lifecycle operation must be confined and explicitly guarded;
- repeated matches must be refused instead of guessed;
- an expected pre-image, candidate identity, path boundary or effect budget should be enforced;
- the caller needs proof of the observed and committed bytes; or
- preserving line endings, comments and unrelated bytes matters.

Do not route a task through Threadmoth merely because it can write a file. Use specialist tooling when the task itself is compilation, testing, Git, package management, formatting or semantic infrastructure planning.

If a requested structured/syntax provider refuses, do not silently convert the operation into text, regex, patch or desired-state mutation. A weaker fallback must be explicit.

## Discover locally

Before relying on a capability, query the installed runtime:

```text
threadmoth --version
threadmoth doctor --json
threadmoth capabilities
threadmoth capabilities --for PATH --json
threadmoth inspect PATH --json
threadmoth suggest PATH
threadmoth schema
threadmoth examples
```

Threadmoth 1.9.1 discovery classifies targets as `structured`, `syntax`, `region`, `exact` or `opaque` and reports preservation level plus explicit fallback routes. The Target Registry is also authoritative for CLI and MCP shorthand provider selection, including special filenames such as `setup.cfg` and `.env.local`.

Use the canonical `threadmoth` executable. `thm` may exist as a convenience alias, but it is not the compatibility contract. Do not assume a `.thm` source extension.

## Make a bounded request

Construct a request with a workspace-relative `file_path`, stable `request_id`, a supported protocol version, explicit provider/operation and cardinality. Add `expected_pre_hash`, a narrow region/candidate guard and hard effect budget when they are known. Prefer an exact path or named target over a broad replacement. Read the local schema and examples instead of inventing fields.

For an ordinary text replacement, the current request shape is:

```json
{
  "version": "1.3.1",
  "request_id": "change-unique-id",
  "file_path": "config.txt",
  "cardinality": { "type": "exactly_one" },
  "budget": { "max_files": 1, "max_matches": 1 },
  "operation": {
    "provider": "text",
    "operation": { "type": "replace", "target": "old", "replacement": "new" }
  }
}
```

Older supported protocol versions remain valid where the installed runtime advertises them. Prefer runtime `schema`/`examples` over hard-coding assumptions.

For a structured scalar value, send JSON with its intended type. The CLI
`set-value` command parses values strictly as JSON; use `set-value ... --string`
or `set-string ...` for a literal UTF-8 string. MCP callers send native JSON
values directly. Invalid JSON is refused rather than silently coerced.

## Preview, then mutate

When the target, cardinality, effect or preservation result is not already obvious, preview first:

```text
threadmoth preview --request request.json
threadmoth mutate --request request.json
```

For a reviewable prepare/commit handoff:

```text
threadmoth plan --request request.json --output plan.json
threadmoth explain --plan plan.json
threadmoth apply-plan --plan plan.json
```

Plans are untrusted and staleable. Applying one rechecks hashes, containment, exact edits, budgets, candidate guards and bounded assertions. Never repair a stale plan by fuzzy relocation; prepare a new plan.

The default mutation output is JSON. `--summary` is a compact human view; keep the full JSON certificate for agent state, audit and follow-up decisions. Transactions can be previewed with `threadmoth transact --request transaction.json --preview`.

Inspect the preview before mutating. Confirm outcome, provider, path, cardinality, changed ranges, effect-budget usage, preservation facts and source identity. Do not treat process exit alone as proof that intended bytes landed.

## Handle outcomes deliberately

- `APPLIED` means the guarded mutation was committed and certified.
- `NO_CHANGE` means the requested result already held; keep the certificate.
- `REFUSED` means Threadmoth found ambiguity, stale identity, unsupported input, a path/safety violation, invalid data or an effect outside the budget.
- `FAILED` means a runtime or commit failure; preserve failure evidence and inspect recovery state when relevant.

Exit codes are stable: `0` for applied/no-change, `2` for refusal and `3` for runtime failure.

On refusal, read the reason code, candidate context and deterministic remedies. Use `threadmoth explain REASON_CODE` and, where provided, `threadmoth suggest --from-refusal CERTIFICATE`. Narrow the request, explicitly choose a reported candidate/route, ask the user to disambiguate, or stop. Never silently widen the edit or bypass a refusal with a raw write.

For transaction failures, use recovery discovery rather than deleting evidence:

```text
threadmoth recover --list
threadmoth recover --inspect TRANSACTION_ID
threadmoth recover --transaction TRANSACTION_ID
```

Report the certificate or refusal reason in the task result. If a fallback was necessary, state why the requested Threadmoth capability did not apply and what broader tool was used.
