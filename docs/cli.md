# Threadmoth CLI

Threadmoth 1.9.1 uses one structured command grammar for parsing, help, validation, completion, and manpage generation.

## Discover before editing

For an unfamiliar target, start with:

```text
threadmoth capabilities --for PATH --json
threadmoth inspect PATH
threadmoth suggest PATH --goal GOAL --at SELECTOR
```

Path-scoped discovery reports the detected target kind, provider, detection basis, confidence class, alternatives, understanding level, preservation level and explicit fallback routes. A fallback being reported does not authorize Threadmoth to silently downgrade a structured/syntax request.

The 1.9 understanding levels are `structured`, `syntax`, `region`, `exact` and `opaque`. See [Coverage](coverage.md).

## Context-economy inspection

The default inspect response is the unchanged identity-only JSON contract. For
large source files, request a deterministic bounded outline and expand only a
selected exact observation:

```text
threadmoth inspect PATH --outline
threadmoth inspect PATH --outline --max-entries 32
threadmoth inspect PATH --expand HANDLE --max-bytes 8192
```

Outline entries contain a bounded label, exact byte/line range, syntax kind and
an observation handle. Handles are read identities bound to the current file
hash, path, provider, language and exact range. They are not mutation
authorization; a changed file causes expansion to refuse rather than relocate
the observation. Unsupported providers return an honest unavailable outline,
and malformed syntax fails closed.

## Plans, assertions, and updates

Prepare without writing, inspect the deterministic artifact, then apply it only if the workspace still matches:

```text
threadmoth plan --request request.json --output plan.json --summary
threadmoth explain --plan plan.json
threadmoth apply-plan --plan plan.json --summary
```

Plan input may include a top-level `assertions` array containing `file_exists`, `file_absent`, `sha256`, or bounded `literal_count` assertions. Assertions run against prospective in-memory state before writes and against landed bytes after commit. A stale or tampered plan returns `PLAN_STALE`/`PLAN_INVALID` and writes nothing.

The updater is an explicit maintenance operation:

```text
threadmoth update --check
threadmoth update
threadmoth update --yes --json
```

It has no arbitrary URL/source flags and is not exposed through MCP. `threadmoth doctor` reports local installation provenance and build metadata without performing a network check.

The latest published release can lag the source version on `main`; `update` follows published GitHub Releases, not unreleased repository source.

## Benchmark commands

The canonical benchmark surface is:

```text
threadmoth benchmark
threadmoth benchmark --quick
threadmoth benchmark --tough
threadmoth benchmark --torture
```

Short forms:

```text
threadmoth benchmark -q
threadmoth benchmark -t
threadmoth benchmark -x
```

Add `--json` (or `-j`) for machine-readable output. Without `--json`, benchmark and torture use the compact human table and final PASS/FAIL summary.

For compatibility, Threadmoth still accepts:

```text
threadmoth benchmark tough
threadmoth torture
```

New documentation and automation should prefer the canonical flag forms.

## Mutation output

Mutation commands return the full JSON certificate by default:

```text
threadmoth preview --request request.json
threadmoth mutate --request request.json
threadmoth transact --request transaction.json --preview
```

For a compact human view, add `--summary` where supported:

```text
threadmoth preview --request request.json --summary
threadmoth mutate --request request.json --summary
threadmoth transact --request transaction.json --preview --summary
```

Desired-state requests use the explicit `desired_state` provider and carry desired bytes as data. Preview reports the derived regions and effect budget before any write; mutate repeats the guarded plan and verifies the landed desired hash.

Recovery discovery is read-only:

```text
threadmoth recover --list
threadmoth recover --inspect TRANSACTION_ID
threadmoth recover --transaction TRANSACTION_ID
```

The summary shows outcome, provider, effect size, preservation facts, hashes and budget status without dumping the bounded diff. If a declared effect budget is too small, recovery guidance reports observed dimensions; Threadmoth never enlarges the caller's budget automatically.

## Provider naming

`filesystem` is the canonical lifecycle provider name in capabilities, schema output, certificates and new requests:

```json
{
  "provider": "filesystem",
  "operation": {
    "type": "create_file",
    "expected_absent": true,
    "content": [104, 105, 10]
  }
}
```

Threadmoth 1.9.1 continues to accept the older request spelling `"provider":"file"` as a compatibility alias. When serialized or described by Threadmoth, the provider is canonicalized to `filesystem`.

## Safe shorthands

Common safe shorthands compile into ordinary typed requests and use the same guarded Core:

```text
threadmoth replace-exact FILE OLD NEW
threadmoth set-value FILE PATH JSON_VALUE [--string]
threadmoth set-string FILE PATH STRING_VALUE
threadmoth create-file FILE CONTENT
```

`set-value` supports registry-selected JSON, JSONC, TOML, YAML/YML, INI and dotenv targets, including registered special filenames such as `setup.cfg`, `tox.ini`, `pytest.ini`, `.editorconfig` and `.env.*`. Values are strict JSON by default. `--string` treats the argument as a literal UTF-8 string without type inference; `set-string` is a convenience alias for the same operation. Invalid JSON remains a refusal.

Each shorthand uses conservative one-file/one-target/one-region budgets and still refuses ambiguity, stale state, unsupported structure or an effect outside its authorization.

## MCP

The MCP stdio server exposes the same mutation authority through typed tools:

```text
threadmoth_capabilities
threadmoth_inspect
threadmoth_suggest
threadmoth_explain
threadmoth_preview
threadmoth_plan
threadmoth_apply_plan
threadmoth_transact_preview
threadmoth_mutate
threadmoth_transact
threadmoth_exact_replace
threadmoth_set_value
```

Preview runs the guarded planning/certification pipeline with commit disabled. `threadmoth_set_value` uses the same Target Registry and shared value-operation resolver as the CLI shorthand, with native JSON values distinguishing strings, booleans and numbers. Self-update remains CLI-only.

JSON-RPC notifications, including `notifications/initialized`, are consumed without a response; ordinary requests receive a JSON-RPC result or standard error response.

## Shell completion

Threadmoth generates completion from the same CLI grammar used to parse commands:

```text
threadmoth completions powershell
threadmoth completions bash
threadmoth completions zsh
threadmoth completions fish
```

The generated script should be installed using the normal mechanism for the target shell. Threadmoth deliberately prints completion rather than silently rewriting shell startup files.

### PowerShell

For the current session:

```powershell
threadmoth completions powershell | Out-String | Invoke-Expression
```

For persistent setup, save the generated completion script somewhere stable and source it from your PowerShell profile.

### Bash

For the current session:

```bash
source <(threadmoth completions bash)
```

For persistent setup, save the output in your normal Bash completion directory or source it from shell configuration.

### zsh

Generate the zsh completion file and place it in a directory on `fpath`, then refresh completion with `compinit`.

### fish

Save the output as `threadmoth.fish` in your normal fish completions directory.

## Help

All subcommands support generated help. High-frequency commands include concrete examples in long help:

```text
threadmoth --help
threadmoth mutate --help
threadmoth preview --help
threadmoth benchmark --help
threadmoth capabilities --help
```

The help-search surface remains available:

```text
threadmoth help mutate
threadmoth help --find refusal
```

Because command names, flags and enumerated values are parsed by `clap`, invalid input gets structured usage errors and close-match suggestions instead of a generic unknown-command fallback.

## Path-aware arguments

Arguments representing files are marked as path values so completion systems can offer filesystem candidates for commands such as:

```text
threadmoth mutate --request <TAB>
threadmoth preview --request <TAB>
threadmoth suggest <TAB>
threadmoth inspect <TAB>
threadmoth capabilities --for <TAB>
```

## Man page

Generate the main roff man page:

```text
threadmoth manpage > threadmoth.1
```

Or write it directly:

```text
threadmoth manpage --output threadmoth.1
```

Threadmoth prints generated artifacts rather than silently editing shell/system configuration.

## Doctor

`threadmoth doctor` reports runtime information, installation provenance, build flavour metadata and CLI usability hints.

```text
threadmoth doctor
threadmoth doctor --json
```

## Compatibility policy

Threadmoth 1.9.1 keeps important pre-1.3 command/provider spellings as compatibility routes, including `apply`, `dry-run`, positional benchmark profiles, `torture`, `transaction-preview`, and request provider alias `file`. It accepts protocol 1.1.0, 1.2.0 and 1.3.0 requests with their promised semantics while advertising protocol 1.3.1 as current.

## Plan review

`threadmoth explain --plan plan.json --format diff` and `threadmoth explain --plan plan.json --format markdown` are read-only review renderers. They include operation, hashes, fresh/stale state and bounded before/after diff; they never apply the plan.
