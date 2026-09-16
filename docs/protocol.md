# Protocol v1.3

Threadmoth 1.10.0 advertises protocol **1.3.1**. Requests using protocol `1.3.0`, `1.2.0`, and `1.1.0` remain accepted with their promised semantics.

A request is JSON with a stable `request_id`, workspace-relative `file_path`, optional namespace and source-identity guards, explicit cardinality, an optional hard effect budget, and one typed provider operation. Unknown fields are rejected.

Run `threadmoth help`, `threadmoth examples`, `threadmoth schema`, `threadmoth explain`, `threadmoth suggest`, or `threadmoth capabilities` for the exact local contract rather than copying an old request shape from documentation.

## Request shape

The operation is encoded as an outer provider and nested tagged operation:

```json
{
  "version": "1.3.1",
  "request_id": "example-2",
  "file_path": "config.json",
  "cardinality": { "type": "exactly_one" },
  "budget": {
    "max_files": 1,
    "max_matches": 1,
    "max_changed_lines": 4
  },
  "operation": {
    "provider": "json",
    "operation": {
      "type": "set",
      "path": "$.server.port",
      "value": 8080
    }
  }
}
```

The current provider family includes:

- `text`: exact and idempotent text operations;
- `json` / `jsonc`: source-range structural paths;
- `toml`: dotted structural paths;
- `yaml`: conservative nested dotted/sequence-index paths;
- `ini`: source-preserving section/key paths;
- `dotenv`: guarded key/value lines;
- `markdown`: bounded document regions;
- `pattern`: bounded regex operations;
- `patch`: exact unified-diff application with no fuzzy relocation;
- `code`: Tree-sitter syntax-node targeting;
- `web`: HTML/CSS/XML syntax-node targeting;
- `desired_state`: exact desired bytes with bounded derived edits;
- `filesystem`: guarded create/delete/rename/move.

`filesystem` is the canonical lifecycle-provider spelling. The older request spelling `file` remains accepted as a compatibility alias and is canonicalized when Threadmoth serializes it.

Structured providers do not silently fall back to text, regex, patch or desired-state mutation. If a provider cannot prove its contract, Threadmoth refuses and may advertise explicit weaker routes for the caller to choose.

## Prepared plans

Prepared plans use schema **1.0**. Their `protocol_version` is inherited from the request or transaction that created the plan, rather than being hard-coded to 1.3.0. Applying a plan accepts only protocol versions advertised by the running binary.

A prepared plan contains exact pre-image hashes, deterministic plan identity, provider-resolved byte edits, budgets, and optional bounded assertions. It is untrusted input: apply rechecks workspace containment, schema version, plan identity, protocol compatibility, provider guard compatibility, hashes, edits, budgets, and postconditions.

Plan input can include bounded assertions:

```text
file_exists
file_absent
sha256
literal_count
```

Assertions are checked against prospective in-memory state before commit and against landed bytes after commit.

## Candidate selection and stale state

Ambiguous candidates can include byte offsets, exact target text, node kind, context, source anchor hash, and deterministic `selection_id`.

A `candidate_guard` is valid only with the exact observed `expected_pre_hash`. If the source changed, Threadmoth returns stale identity rather than relocating the candidate somewhere similar.

A prepared plan has the same philosophy: changed pre-images produce `PLAN_STALE`; tampered/invalid plan structure produces `PLAN_INVALID` or another typed refusal. No fuzzy relocation or trusted-plan bypass exists.

## Outcomes and evidence

Outcomes are:

```text
APPLIED
NO_CHANGE
REFUSED
FAILED
```

An applied certificate includes protocol/provider identity, request ID, expected and observed cardinality, pre/post SHA-256, changed byte and line ranges, bounded diff, structural validation, preservation facts, effect-budget usage, commit guarantee, and recovery state. Desired-state certificates additionally record desired-state proof information.

Transaction certificates contain one certificate per member plus transaction rollback/recovery state.

Every refusal and relevant failure certificate includes a stable `reason_code`. Use:

```text
threadmoth explain REASON_CODE
threadmoth suggest --from-refusal CERTIFICATE
```

for local deterministic recovery guidance. Threadmoth reports choices; it does not choose an ambiguous target on the caller's behalf.

The full JSON certificate remains the default mutation output. Humans can add `--summary` to supported commands for a compact view without changing mutation semantics.

Exit codes remain:

```text
0  applied / no change
2  refused
3  runtime failure
```

## Discovery metadata in 1.9

Path-scoped discovery can report the target kind, provider, detection basis, confidence class, alternatives, understanding level, preservation level, and explicit fallback routes. These are discovery facts, not permission to silently weaken a mutation request.

For the authoritative local schema and capability fingerprint, use:

```text
threadmoth schema --json
threadmoth capabilities --json --all
```
