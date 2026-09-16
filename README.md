<p align="center">
  <img src="assets/threadmoth-icon-light.svg" alt="Threadmoth" width="180" />
</p>

<h1 align="center">Threadmoth</h1>

<p align="center"><strong>Deterministic file mutation for AI agents.</strong></p>

<p align="center">Find the exact thing. Change only that thing. Prove what happened.</p>

---

AI is good at deciding **what should change**.

The dangerous part is what it has to do last: actually change the files.

Coding agents routinely fall back to direct writes, regex replacement, patch tools, shell commands and one-off scripts. Each has different behaviour around ambiguity, stale files, formatting, path safety and failure. An agent can make the right decision and still land the wrong edit.

**Threadmoth puts one deterministic boundary between AI intent and your source tree.**

The model decides the change.

Threadmoth decides whether that exact change can be made safely.

If it can, Threadmoth applies it and proves what landed.

If it cannot, it refuses.

```text
AI intent
   ↓
Threadmoth
   ↓
OBSERVE
IDENTIFY
GUARD
PLAN
VERIFY
MUTATE
VERIFY
CERTIFY
   ↓
Source tree
```

## Why this is needed

Most editing tools were built for humans.

Humans notice when a replacement hit the wrong function, a formatter rewrote half a file, a patch landed somewhere unexpected, or another process changed the file underneath them.

Autonomous agents need those assumptions turned into machinery.

Threadmoth gives them that machinery.

### Threadmoth will not silently:

- choose the first of several matches;
- relocate a stale edit somewhere "close enough";
- rewrite unrelated formatting;
- follow a path outside the workspace;
- ignore a changed pre-image;
- exceed the authorised effect budget;
- report success before reading the result back.

Ambiguity is not something Threadmoth tries to be clever about.

**Ambiguity is a refusal.**

That is the point.

---

## The core idea

Parsers are allowed to **locate** structure.

They are not allowed to rewrite the file.

> The parser gets to point at the cloth. It doesn't get to re-weave it.

Providers identify exact byte ranges. Threadmoth Core owns the mutation.

That means a structured edit does not require parsing a file, modifying an AST and printing the whole thing back out.

Unrelated bytes stay unrelated.

Comments stay comments.

Line endings stay line endings.

Formatting outside the authorised edit stays untouched.

---

## What you get

### Deterministic targeting

The same request against the same bytes produces the same decision.

### Refusal-first safety

Two valid targets means two candidates, not an arbitrary winner.

### Source preservation

Only authorised byte ranges change.

### Stale-state protection

Requests and prepared plans are tied to the exact bytes they were created against.

### Effect budgets

A caller can bound files, matches, changed regions, lines and bytes.

### Postconditions

A mutation can require facts to be true afterwards, such as:

```text
file exists
file is absent
SHA-256 matches
literal occurs exactly N times
```

These are checked before commit against the prospective state and again after the committed bytes are read back.

### Transactions and recovery

Multi-file changes are staged, journalled and verified as one guarded operation, with recovery state for interrupted commits.

### Proof

A successful mutation returns evidence including:

```text
pre-image hash
post-image hash
changed ranges
effect usage
structural validation
commit verification
```

Success means more than "the write call returned OK."

It means the requested change was observed, bounded, committed, read back and verified.

---

## Prepared plans

Threadmoth can separate **deciding an exact mutation** from **committing it**.

```bash
threadmoth plan --request request.json --output plan.json
threadmoth explain --plan plan.json
threadmoth apply-plan --plan plan.json
```

For supervisor or human review, `threadmoth explain --plan plan.json --format diff` and `--format markdown` provide read-only plan summaries with files, operations, hashes, assertions, effect size, and current stale/fresh state.

A plan contains the resolved targets, expected pre-images, exact edits, budgets and assertions.

It can be inspected by another agent or a human before anything is written.

When applied, the plan is treated as untrusted input. Threadmoth re-reads reality and re-checks everything.

If the repository changed:

```text
PLAN_STALE
```

No fuzzy relocation. No automatic repair. No guessing.

---

## What Threadmoth can target

| Provider | Purpose |
|---|---|
| **Text** | Exact replacement, byte ranges and idempotent exact-text operations |
| **JSON / JSONC** | Source-preserving structural value edits |
| **TOML** | Structure-aware dotted-key edits |
| **YAML** | Conservative nested map/sequence scalar edits with source-local preservation |
| **INI** | Source-preserving section/key edits for `.ini`, `setup.cfg`, `tox.ini`, `pytest.ini` and `.editorconfig` |
| **Markdown** | Heading/list/fenced-region bounded changes |
| **dotenv** | Key edits while preserving comments and surrounding lines |
| **Pattern** | Bounded regex operations |
| **Patch** | Exact unified-diff application with no fuzzy relocation |
| **Code** | Tree-sitter structural targeting |
| **Web** | Structural HTML, CSS and XML targeting |
| **Filesystem** | Guarded create, delete, rename and move |
| **Desired state** | Bounded deterministic change from observed bytes to supplied desired bytes |

### Syntax-aware source coverage

Programming and declarative syntax:

```text
JavaScript
TypeScript
Python
Rust
Go
C
C++
Java
C#
PHP
Bash / Shell
PowerShell
SQL
HCL / Terraform syntax
```

Syntax variants:

```text
JSX
TSX
```

Web / markup syntax:

```text
HTML
CSS
XML
```

PHP includes PHP source and embedded PHP parsing. HCL support is source syntax only; Threadmoth does not claim Terraform resource, provider, state or dependency semantics.

Unsupported languages do not make an ordinary UTF-8 text file unusable. Threadmoth can still offer explicit exact text, strict patch, bounded pattern or desired-state routes where applicable. It never silently downgrades a structured request to a weaker route.

See [Coverage](docs/coverage.md) for the full 1.9 model and current refusal boundaries.

Providers locate candidates.

**Core commits them.**

---

## Built for agents

Threadmoth is designed to be discovered rather than memorised.

Start here:

```bash
threadmoth capabilities
threadmoth capabilities --for PATH --json
threadmoth suggest PATH
threadmoth inspect PATH
threadmoth inspect PATH --outline
threadmoth inspect PATH --expand HANDLE
```

Then:

```bash
threadmoth preview --request request.json
threadmoth mutate --request request.json
```

For multi-file work:

```bash
threadmoth transact --request transaction.json --preview
threadmoth transact --request transaction.json
```

For prepared work:

```bash
threadmoth plan --request request.json --output plan.json
threadmoth explain --plan plan.json
threadmoth apply-plan --plan plan.json
```

When Threadmoth refuses something:

```bash
threadmoth explain REASON
threadmoth suggest --from-refusal refusal.json
```

Machine-facing mutation output is structured JSON. Diagnostics stay separate. Exit behaviour distinguishes success/no-change, refusal and runtime failure.

---

## MCP

Threadmoth also runs as a native MCP stdio server:

```bash
threadmoth mcp
```

Minimal configuration:

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

The MCP interface exposes the same guarded Core used by the CLI. It does not get a weaker safety path.

---

## OpenAI and Codex

Threadmoth is designed for AI coding agents. When an agent needs a precise bounded file mutation, its packaged skill and local MCP interface can provide a deterministic mutation boundary rather than a direct write or ad-hoc script.

OpenAI packaging and platform limits are documented in [docs/openai-integration.md](docs/openai-integration.md).

---

## A better last mile for coding agents

Without Threadmoth, an agent's mutation layer often looks like this:

```text
sed
regex
apply_patch
PowerShell
Python script
direct write
jq / yq
AST rewrite
whatever seemed convenient this time
```

Every path comes with different assumptions.

Threadmoth reduces that surface to:

```text
agent decides
      ↓
one mutation boundary
      ↓
verified source tree
```

Threadmoth does not replace formatters, compilers, test runners or the model. Those tools can decide **what the desired result should be**. Threadmoth exists to make sure the authorised result is the result that actually lands.

---

## What Threadmoth deliberately does not do

Threadmoth is not:

```text
an AI
a coding agent
a formatter
a compiler
a test runner
a shell
a Git client
a build system
a package manager
a Terraform engine
```

It does not invent code. It does not decide which ambiguous target you meant. It does not run arbitrary commands to determine whether an edit was successful.

It does one job:

> **Make bounded file mutation deterministic, inspectable and difficult to get wrong.**

---

## Install

Download the appropriate standalone binary from the **[latest GitHub release](https://github.com/matthewjameswatkins1978-cyber/Threadmoth/releases/latest)** and put `threadmoth` on your `PATH`.

The repository source is version **1.9.1**. The latest-release page is authoritative for which version and artifacts have actually been published; source can be ahead of the most recent release while a release is being prepared.

Check the installation:

```bash
threadmoth --version
threadmoth doctor
threadmoth capabilities
```

Standalone installations can update explicitly:

```bash
threadmoth update --check
threadmoth update
```

The 1.9 release pipeline supports portable Windows x86-64, Linux x86-64, macOS Apple Silicon and macOS x86-64 archives, with checksums and a release manifest. Windows and Linux also have explicit `-v3` modern artifacts built for the `x86-64-v3` CPU baseline. Portable remains the compatibility default.

Local maximum-performance builds can use `target-cpu=native` through the checked-in build scripts. Native builds are tuned to the machine that compiles them and are never presented as universal downloads. See [Performance builds](docs/performance-builds.md) and [Performance results](docs/performance-results.md).

For common safe edits, these small commands compile into the same guarded Core pipeline as canonical requests:

```bash
threadmoth replace-exact config.txt OLD NEW
threadmoth set-value config.json $.server.port 8080
threadmoth set-value setup.cfg metadata.version 0.2.0 --string
threadmoth set-string .env.local API_HOST 127.0.0.1
threadmoth create-file notes.txt "managed file"
threadmoth doctor --json
```

`set-value` parses its value strictly as JSON. Use `--string` or `set-string` when the intended value is a literal string; Threadmoth does not silently coerce invalid JSON. The Target Registry is the canonical classifier for extensions, special filenames and filename families, and the CLI and MCP shorthands use the same resolver.

Shorthands authorize one file, one target and one changed region. They refuse ambiguity and stale state. When a refusal includes deterministic `recovery` remedies, `threadmoth suggest --from-refusal refusal.json` emits complete guarded next-request templates; the caller still chooses among candidates.

Before choosing an edit route, agents can inspect Threadmoth's coverage model with `threadmoth capabilities --for PATH --json` or `threadmoth inspect PATH`. For large syntax-aware files, `inspect --outline` returns a compact deterministic map and `inspect --expand HANDLE` returns one bounded exact region after rechecking the source hash. Files are classified as structured, syntax-aware, bounded regions, exact text, or opaque/refused. Unknown valid UTF-8 remains available through explicit exact mutation; binary and unsupported encodings do not silently fall back.

The current protocol is **1.3.1** and remains compatible with 1.3.0, 1.2.0 and 1.1.0 requests. Mutation, plan application, transaction, filesystem lifecycle and recovery writes use a bounded cooperating-process workspace lock and refuse with `WORKSPACE_BUSY` rather than waiting indefinitely. Unrelated external writers remain covered by stale-state and landed-byte verification, but are not controlled by that lock.

Mutation operations remain local. The explicit update command is the only normal Threadmoth operation that needs network access.

To build from source:

```bash
git clone https://github.com/matthewjameswatkins1978-cyber/Threadmoth.git
cd Threadmoth
cargo build --release --locked
```

---

## Test it yourself

Threadmoth ships its own correctness and adversarial checks:

```bash
threadmoth benchmark --quick
threadmoth benchmark --tough
threadmoth benchmark --torture
```

The important benchmark number is not merely speed.

It is:

```text
wrong successful mutations: 0
```

Because a mutation tool that is extremely fast at changing the wrong thing is not extremely useful.

---

## The contract

Threadmoth's contract is deliberately simple:

```text
If the requested mutation is exact, bounded and still valid:
    apply it
    verify it
    prove it

Otherwise:
    refuse
```

That is Threadmoth.
