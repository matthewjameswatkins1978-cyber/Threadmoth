# Threadmoth documentation

Threadmoth is a fast, deterministic structural search-and-rewrite runtime for AI agents. The main [README](../README.md) is the best place to start; this page maps the current technical documentation.

## Start here

For Threadmoth 1.10.0, start with the [coverage model](coverage.md), [target registry](target-registry.md), [protocol](protocol.md), [context-economy evidence](context-economy-results-1.10.md), and [performance builds](performance-builds.md). Measured local performance evidence is recorded separately in [performance results](performance-results.md).

| Document | Purpose |
|---|---|
| [Coverage model](coverage.md) | The 1.10.0 structured, syntax, region, exact and opaque capability ladder |
| [Target registry](target-registry.md) | Canonical file detection and capability metadata |
| [CLI guide](cli.md) | 1.10.0 commands, coverage discovery, bounded inspect views, plans, assertions, updater, shorthands, recovery, completion and manpages |
| [Agent integration](agent-integration.md) | Minimal instructions and safe usage flow for coding agents and MCP clients |
| [OpenAI integration](openai-integration.md) | Codex packaging, local MCP setup, platform status and submission boundary |
| [Architecture](architecture.md) | Core mutation authority, providers, target discovery, plans and verification |
| [Protocol](protocol.md) | Request/response contract and machine-facing behaviour |
| [Provider contract](provider-contract.md) | Rules every mutation provider must obey |
| [Syntax targeting](syntax-targeting.md) | AST-grounded and AST-typed source-preserving edits |
| [Desired state](desired-state.md) | Deterministic desired-state planning and verification |
| [Threat model](threat-model.md) | What Threadmoth protects against and what it deliberately does not do |
| [Benchmark report](benchmark-report.md) | Correctness-first benchmark method and retained performance evidence |
| [Performance builds](performance-builds.md) | Portable, modern x86-64-v3, native and PGO build policy |
| [Performance results](performance-results.md) | Measured local build-flavour results retained from the 1.9 line |
| [What Threadmoth replaces](what-threadmoth-replaces.md) | The last-mile mutation role Threadmoth is designed to consolidate |
| [Agent challenge](agent-challenge.md) | Reproducible field test for agent discovery and refusal recovery |
| [Distribution ledger](distribution-ledger.md) | Timestamped operational history of releases, adapters and outreach |

## Historical design records

The following files are intentionally retained as historical records. They describe the product at the named milestone and are not the source of truth for current 1.10.0 capabilities:

- [v1.0 acceptance boundary](v1-acceptance.md)
- [v1.1 discovery surface](v1.1-discovery.md)
- [optimization pass 1](optimization-plan-pass1.md)

For current behaviour, prefer `threadmoth capabilities --json --all`, `threadmoth schema --json`, and the 1.10.0 documents above.

## Core idea

Threadmoth does not ask an agent to be careful while performing an unconstrained edit. It narrows the edit itself.

```text
OBSERVE -> IDENTIFY -> GUARD -> PLAN -> VERIFY PROSPECTIVE -> MUTATE -> VERIFY COMMITTED -> CERTIFY
```

A provider may identify and propose a candidate mutation, but **Core alone commits**. If identity is ambiguous, reality changed since observation, the request exceeds its bounds, or validation fails, the operation is refused rather than guessed.

## Performance

Threadmoth includes correctness-checked benchmark and torture modes:

```text
threadmoth benchmark
threadmoth benchmark --tough
threadmoth benchmark --torture
```

Treat reported timings as local measurements, not universal platform claims. The correctness signal remains the important one: a benchmark fails if an expected successful mutation lands the wrong bytes.

## Useful CLI discovery

```text
threadmoth help
threadmoth doctor --json
threadmoth capabilities
threadmoth capabilities --for PATH --json
threadmoth schema
threadmoth examples
threadmoth suggest PATH
threadmoth inspect PATH
threadmoth explain REASON_CODE
threadmoth completions powershell
threadmoth manpage
threadmoth mcp
```

For machine integration, mutation output is JSON on stdout, diagnostics are on stderr, and stable exit codes distinguish success/no-change, refusal and runtime failure.

## Design rule

The important contract is not merely that Threadmoth can rewrite a file. It is that an `APPLIED` result carries enough evidence to say what bytes were observed, what edit was authorised, what validation ran, and what bytes were actually committed.
