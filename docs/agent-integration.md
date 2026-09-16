# Agent integration

Threadmoth is designed to be discovered by an agent rather than memorised by one.

For most coding-agent setups, start with this minimal instruction:

```text
Threadmoth is installed and available for deterministic, source-preserving file mutation.
Prefer it for bounded structural edits when its capabilities apply.
Discover usage with:
  threadmoth --help
  threadmoth capabilities
  threadmoth capabilities --for PATH
  threadmoth suggest PATH
Preview before committing when uncertain.
Do not bypass a Threadmoth refusal with a broader raw edit unless the user explicitly authorizes it.
```

That is intentionally small. The point is to test Threadmoth's discovery surfaces rather than preload the model with its protocol.

Before constructing a mutation request, apply the canonical ambiguity rule from `skills/threadmoth/SKILL.md`: **preserve ambiguity when shaping mutation requests; do not add identifying information that was not supplied by the user or established by evidence. Narrow from evidence, never from imagination.** If multiple plausible targets remain, inspect or suggest candidates and ask the user to choose, or stop with a clear refusal.

## 1.9 coverage discovery

Before choosing a mutation route for an unfamiliar path, prefer:

```text
threadmoth capabilities --for PATH --json
threadmoth inspect PATH --json
threadmoth suggest PATH --goal GOAL --at SELECTOR
```

The path-scoped result describes the detected target kind, provider, detection basis, confidence class, understanding level, preservation level, alternatives and explicit fallback routes. A weaker route being listed is not permission to use it automatically.

Threadmoth 1.9 distinguishes structured formats, parser-grounded syntax, bounded regions, exact text and opaque/refused content. Unsupported source languages may still be safely editable as exact text; binary/invalid text is not silently accepted.

## Antigravity

The repository root contains a native Antigravity [`plugin.json`](../plugin.json) that packages the existing `skills/threadmoth/SKILL.md` without duplicating its instruction text. Install it with `agy plugin install <repository>`.

The adapter should be judged by live behaviour rather than the existence of the manifest. Record whether the agent discovers Threadmoth, previews/guards an edit, respects refusals and returns the final certificate.

## OpenAI / Codex

The repository also contains the documented OpenAI-native [`.codex-plugin/plugin.json`](../.codex-plugin/plugin.json), [`.mcp.json`](../.mcp.json), and [Codex marketplace entry](../.agents/plugins/marketplace.json). These package the existing `skills/threadmoth/SKILL.md` and the existing `threadmoth mcp` server; they do not create a second mutation implementation or install the binary.

Use [the OpenAI integration guide](openai-integration.md) for setup and the separate status of Codex local, ChatGPT Desktop, ChatGPT Web, managed workspaces, and public submission.

## Useful discovery commands

```text
threadmoth --help
threadmoth capabilities
threadmoth capabilities --for PATH
threadmoth examples
threadmoth schema
threadmoth suggest PATH
threadmoth explain REASON_CODE
```

## Recommended agent policy

An agent should:

1. inspect path-scoped capabilities before guessing a provider or request shape;
2. use `suggest` for unfamiliar files or formats;
3. preview when the intended effect is not obvious, or use a prepared plan when work must cross an agent step or human review boundary;
4. treat `REFUSED` as information, consume its machine-readable recovery remedies, and let the caller choose the next request;
5. never silently downgrade a structured/syntax request to text, regex, patch or desired-state mutation;
6. only use a broader fallback when Threadmoth genuinely does not cover the task or the user explicitly authorizes the wider effect;
7. preserve and report the resulting certificate when diagnosing surprising behaviour.

## What Threadmoth is not

Threadmoth is not an AI task planner, formatter, compiler, test runner, Git client, shell, package manager or Terraform engine. Its `plan` command prepares a deterministic guarded mutation artifact; it does not decide what work should be done.

The model decides what should happen. Threadmoth provides a narrow deterministic mutation boundary and proves what actually changed.

## Recovery loop

Certificates may contain `recovery.requires_choice` and bounded remedies. For ambiguity, remedies can include a complete request patch with the exact candidate guard and observed pre-image hash. For stale state, the remedy refreshes observed identity; for effect budgets, it reports the observed minimum dimensions. `suggest --from-refusal` exposes the same deterministic recovery material. Threadmoth never picks a candidate on the caller's behalf.

## MCP

Threadmoth also exposes an MCP stdio adapter:

```text
threadmoth mcp
```

MCP is an adapter over the same deterministic Core. The CLI/JSON contract remains the lowest-common-denominator integration surface. A minimal stdio configuration is:

```json
{
  "mcpServers": {
    "threadmoth": {
      "command": "threadmoth",
      "args": ["mcp"]
    }
  }
}
```

The 1.9 MCP tools include:

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

`threadmoth_set_value` follows the same Target Registry and shared value-operation resolver as the CLI shorthand for JSON, JSONC, TOML, YAML, INI and dotenv targets. Special filenames such as `setup.cfg` and `.env.local` therefore resolve consistently. MCP sends native JSON values, so `true` is a boolean and `"true"` is a string; it does not need a CLI-style `--string` flag. There is deliberately no `threadmoth_update`; self-update is an explicit CLI-only maintenance command.

## The 30-second agent loop

```text
capabilities --for / suggest
        ↓
preview
        ↓
APPLIED or REFUSED
        ↓
if REFUSED: explain / inspect remedies / choose explicit next route
        ↓
preview again with refreshed identity/guard when needed
        ↓
mutate the same guarded request
        ↓
keep the certificate as proof
```

Tiny example conversation:

```text
Agent: threadmoth_inspect({"path":"config.json"})
Threadmoth: {"sha256":"...","encoding":"utf8","newline_profile":"lf",...}
Agent: threadmoth_suggest({"path":"config.json","goal":"set-value","at":"$.port"})
Threadmoth: {"provider":"json","request_template":{...}}
Agent: threadmoth_preview(request_template)
Threadmoth: {"outcome":"APPLIED","commit":{"mode":"dry_run"},...}
Agent: threadmoth_mutate(the_same_request)
Threadmoth: {"outcome":"APPLIED","post_hash":"...",...}
```

When work must cross an agent step or human review boundary, use `threadmoth_plan`/`threadmoth_apply_plan` or the equivalent CLI commands. The plan is portable but not trusted: apply refuses stale pre-images and rechecks guards, budgets, exact edits and postconditions.
