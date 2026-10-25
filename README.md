<p align="center">
  <img src="assets/threadmoth-icon-light.svg" alt="Threadmoth" width="180" />
</p>

<h1 align="center">Threadmoth</h1>
<p align="center"><strong>The editing engine for AI agents that need to get it right.</strong></p>
<p align="center"><strong>Find the exact thing. Change only that thing. Prove what happened.</strong></p>

<p align="center">
  <a href="#start-here">Get started</a> ·
  <a href="#the-toolbelt-specialists-find-threadmoth-writes">The Toolbelt</a> ·
  <a href="#why-threadmoth-exists">Why Threadmoth</a> ·
  <a href="#where-it-shines">Use cases</a> ·
  <a href="#capabilities">Capabilities</a> ·
  <a href="#install">Install</a>
</p>

---

## Brilliant agents deserve a better way to edit files.

AI can reason across a codebase, find an elegant solution and produce a convincing patch. But there is a difference between **knowing what to change** and **proving that the right change landed**.

One ambiguous match. One stale file. One overenthusiastic formatter. One failed write halfway through a multi-file edit. That's enough to turn a good decision into a bad result.

**Threadmoth gives coding agents a dependable last mile.** It is an open-source, MIT-licensed Rust CLI and MCP server for precise, bounded, source-preserving edits. Agents can use exact text, structured-file providers, syntax-aware tools and, as of **1.21.0**, cross-file AST pattern rewrites through the **Toolbelt**. Every supported edit goes through the same guarded mutation pipeline.

| What agents need | What Threadmoth provides |
| --- | --- |
| **Make the intended edit** | Exact targeting, cardinality checks and context-aware providers |
| **Keep everything else intact** | Byte-conscious edits that preserve unaffected source, formatting and line endings where supported |
| **Work across files** | Guarded content transactions, effect budgets, journaling and recovery reporting |
| **Use powerful specialists safely** | ast-grep proposes structural rewrites; Threadmoth validates and applies them |
| **Know what really happened** | Read-back verification, source/result hashes, diffs, structured outcomes and certificates |
| **Avoid bluffing** | Clear refusals for ambiguity, stale inputs, unsupported operations and breached limits |

**Not another coding agent. Not another IDE. Not a wrapper that blindly runs search-and-replace.** Threadmoth is the editing authority that an agent can call when precision matters.

~~~text
      HUMAN + CODING AGENT
       decides the intent
                |
                v
    DISCOVER + INSPECT + PROPOSE
    native providers / ast-grep
                |
                v
       THREADMOTH CORE
   source identity • scope • budgets
   preview • guarded transaction
   verification • recovery
                |
                v
      EXACT EDIT OR REFUSAL
       + machine-readable evidence
~~~

It works with capable agents through a normal CLI or MCP, without choosing their model, orchestrator or workflow for them.

## Start here

For an ordinary text edit, the common case is now one command:

~~~bash
# Single-file editing with inline strings:
threadmoth edit config.txt --search "mode=development" --replace "mode=production"

# For quote-heavy, multiline or shell-sensitive content, bypass quoting with files:
threadmoth edit src/main.rs --search-file old.txt --replace-file new.txt

# Guard multiple replacements (refuses unless target appears exactly N times):
threadmoth edit src/main.rs --search "old_call()" --replace "new_call()" --expect-count 3

# Or replace all occurrences across the file (subject to effect budgets):
threadmoth edit src/main.rs --search "DEBUG" --replace "INFO" --all

# For mixed-newline files, provide exact bytes and explicit policy:
threadmoth edit src/main.rs --search-file old.txt --replace-file new.txt --newline-policy exact
~~~

Threadmoth checks that the exact target is unambiguous and permitted, prepares the change, applies it through the guarded Core, reads back the result and returns a certificate. There is no mandatory intermediate plan file or second application command.

To see the proposed edit **without modifying the source**:

~~~bash
threadmoth edit config.txt --search "mode=development" --replace "mode=production" --plan
~~~

To get machine-readable output, add `--json`.

For multiline edits and several files, use a compact edit document:

~~~text
*** File: frontend.txt
<<<
VERSION=1.0.0
===
VERSION=2.0.0
>>>

*** File: backend.txt
<<<<<<< SEARCH
VERSION=1.0.0
=======
VERSION=2.0.0
>>>>>>> REPLACE
~~~

Save that as `changes.tm`, then run:

~~~bash
threadmoth edit changes.tm
# Or: threadmoth edit changes.tm --plan
# Or pipe the document into: threadmoth edit -
~~~

The document is prepared as one guarded content transaction. The source bytes and the certificate, not the agent's optimism, determine success.

A file's format matters. For JSON, TOML, YAML and other recognised structured formats, the general exact-text route may refuse rather than compromise source-preservation guarantees. Discover the right provider or use a structured shorthand:

~~~bash
threadmoth suggest config.json
threadmoth capabilities --for config.json
threadmoth set-value config.json '$.server.port' 8080
threadmoth set-string .env.local API_HOST 127.0.0.1
~~~

`set-value` treats its value as JSON; `set-string` takes a literal string. No silent type guessing.

### Cross-file structural editing with the Toolbelt

Threadmoth 1.21.0 brings **ast-grep** into the same dependable editing workflow. Match an AST pattern across a code tree, preview the proposed rewrites, and let Threadmoth handle validation, mutation and verification. ast-grep is invoked to **propose**, not to write source files directly:

~~~bash
# Preview proposed AST transformations across files without touching disk:
threadmoth structural --pattern 'old_fn($A)' --rewrite 'new_fn($A)' src/ --preview

# Prepare a reviewable plan artifact:
threadmoth structural --pattern 'old_fn($A)' --rewrite 'new_fn($A)' src/ --plan plan.json
threadmoth explain --plan plan.json --format diff
threadmoth apply-plan --plan plan.json --summary

# Or apply directly with atomic verification and rollback protection:
threadmoth structural --pattern 'old_fn($A)' --rewrite 'new_fn($A)' src/
~~~

## The Toolbelt: specialists find. Threadmoth writes.

The exciting part of **1.21.0** is not simply that Threadmoth can run a structural search. It's that powerful external tools can contribute their understanding **without becoming a second, less accountable way to modify a project**.

The first Toolbelt attachment uses [ast-grep](https://ast-grep.github.io/) to recognise code shapes and propose rewrites, including metavariable-driven transformations over multiple files. Threadmoth then checks the returned paths, UTF-8 byte ranges, matched source, overlapping edits, observed file hashes and effect limits **before** anything is committed.

~~~text
       ast-grep: find the code shape
                    |
             proposal JSON
                    v
         Threadmoth: validate it
                    |
        prepare / preview / guard
                    v
       apply, verify and certify
~~~

One command for routine changes, explicit plans for higher-stakes ones, **one mutation authority underneath both**. Structural proposals inherit Threadmoth's guarded transaction and recovery machinery instead of relying on a successful subprocess exit as proof of a correct edit.

This is **syntax-aware structural editing**, not compiler-level symbol resolution or a promise to update every semantic reference. The subprocess is not an OS sandbox; Threadmoth invokes the attachment in non-mutating proposal mode and applies its own checks before writing.

The Toolbelt contract is intentionally small: specialists advertise their capabilities and propose changes; Threadmoth decides what it can safely commit. See the [Toolbelt architecture](docs/toolbelt-attachment-architecture.md).

## Why Threadmoth exists

Most software-editing commands answer: **“Can I perform this replacement?”**

Threadmoth asks a stricter question:

> **“Can I prove that this particular replacement is the one authorised, against the file that actually exists now, without changing anything else?”**

Those are different jobs.

An ordinary search-and-replace command can happily touch every match. A patch can apply successfully even when the caller's larger assumptions are wrong. An AST-aware rewrite can identify a sophisticated code pattern without needing to provide a transaction certificate. A serializer can make a perfectly valid configuration change while also reformatting the whole file.

Those tools are useful. Threadmoth addresses the narrower but crucial last mile: *turn an already-decided change into a bounded, inspectable, verified mutation.*

## What makes it different

### 1. Let specialist tools find it. Keep one authority for writing it.

A parser may recognise a JSON key, a TOML table, a Markdown heading, an HTML node or a function in a supported language. But it does not get permission to reprint or regenerate the whole file.

**Providers locate and validate. Threadmoth Core decides what may be written.**

Only the proven, authorised byte ranges are changed. Comments, spacing, ordering and line endings outside those ranges are left alone. Where a requested operation cannot preserve the relevant representation, Threadmoth can refuse instead of pretending otherwise.

### 2. Ambiguity is evidence, not an invitation to guess.

If `status = "pending"` occurs twice, Threadmoth does not arbitrarily pick the first one.

It returns a refusal such as `TARGET_AMBIGUOUS`, with grounded candidate evidence when available, so the agent can inspect the file, provide exact surrounding context or explicitly select a justified target. It never invents certainty just to keep moving.

### 3. Stale plans do not get a second life by guesswork.

A prepared edit is tied to the observed source. Application rechecks source identity, operation guards and exact edits. If the file changed in the meantime, the plan refuses with `PLAN_STALE` rather than silently relocating to something “close enough”.

That matters when other agents, developers, formatters or background tools may be touching the same checkout.

### 4. Changes have boundaries.

Requests can limit the number of files, matches, regions, changed lines and bytes, and can constrain allowed paths. Workspace containment, source checks and prospective assertions sit on the mutation path, not only in a prompt.

Threadmoth uses a bounded workspace lock to coordinate cooperating mutators. It does not claim to control unrelated external writers or replace the sandbox or operating-system permissions around an agent.

### 5. A commit has a verifiable result.

A certificate records outcomes and relevant evidence, including source/result hashes, the affected range or effect, guard/budget decisions, and verification details.

Outcomes distinguish `APPLIED`, `NO_CHANGE`, `REFUSED` and `FAILED`. Failure reporting includes recovery state: if rollback needs manual attention, Threadmoth says so rather than declaring the source unchanged.

There is a meaningful difference between *a command returned zero* and *the expected bytes were read back and verified*.

### 6. One easy door, with deeper machinery behind it.

The unified `threadmoth edit` CLI and `threadmoth_edit` MCP tool give routine exact changes a one-call path. More sophisticated transformations can use `threadmoth structural` and `threadmoth_structural`; both feed Threadmoth's guarded execution authority, rather than introducing an independent file writer.

More demanding work still has explicit, inspectable steps:

~~~bash
threadmoth prepare-text config.txt --edit OLD NEW --output plan.json --summary
threadmoth explain --plan plan.json --format diff
threadmoth apply-plan --plan plan.json --summary
~~~

Plans can be persisted for human or supervisor review. A `--plan` preview **without** `--output` is in memory only and does not leave a saved plan behind.

### Request IDs vs Transaction IDs

- **`request_id`**: A transient in-memory correlation identifier for single-file operations and log traces. It is never written to disk, does not act as an idempotency key, and will never cause a collision or refusal on reuse.
- **`transaction_id`**: A durable journal identifier for multi-file content transactions, persisted on disk in `.threadmoth/recovery/<transaction_id>.json`. Reusing an active or unrecovered transaction ID fails safely with an `ALREADY_EXISTS` refusal to prevent overwriting recovery journals.

The CLI, MCP integration, concise orientation, capability discovery, source inspection and contextual refusal guidance are designed to let capable agents teach themselves the tool, without a wizard or a giant mandatory prompt.

## Where it shines

Threadmoth is useful wherever **the edit is small, the consequences aren't, and an agent needs to be able to explain its work**.

| Agent task | Threadmoth's contribution |
| --- | --- |
| **Change one value in a busy configuration file** | Select the correct structured key without gratuitously rewriting surrounding source |
| **Update a repeated code pattern across a project** | Let ast-grep locate syntax shapes; validate proposals and apply a guarded multi-file content transaction |
| **Change a precise part of a long file** | Match with exact text or scoped context; refuse if more than one target fits |
| **Coordinate edits in an active checkout** | Check source identity and reject stale plans instead of guessing after another writer has changed a file |
| **Perform a risky edit in reviewable stages** | Prepare a plan, inspect a diff, apply against the expected source, then read the certificate |
| **Recover from a failed transaction** | Attempt rollback, retain recovery evidence when restoration is incomplete and report what needs attention |

This isn't about replacing the agent's judgement. It's about giving that judgement **a reliable way to touch the filesystem**.

## How it compares

Threadmoth does not claim to win every editing task. These are different tools with different strengths:

| Approach | Where it shines | What Threadmoth adds or does differently |
| --- | --- | --- |
| **`sed`, regex, scripts and direct writes** | Flexible, familiar, powerful for ad-hoc transformations | Exact-target cardinality, stale-source checks, effect budgets, guarded application and result certificates under a common contract |
| **[`git apply`](https://git-scm.com/docs/git-apply) and patch tools** | Applying diffs with context, checking patch applicability, Git-integrated workflows | Provider-aware targeting beyond patches, explicit effect budgets, source-preserving structured edits, and one consistent certificate interface; `git apply --check` remains valuable |
| **[ast-grep](https://ast-grep.github.io/) and [Comby](https://comby.dev/)** | Rich syntactic or structural pattern matching and large-scale rewrites | Threadmoth **now integrates ast-grep** as its first Toolbelt specialist, turning AST match proposals into source-checked, guarded edits with certificates; Comby remains a separate tool |
| **`jq`, `yq` and format-specific writers** | Powerful data queries, transformations and format-aware manipulation | Small source-local edits with a common preservation and verification contract, when supported; Threadmoth is not a substitute for complex format-specific transformations |
| **[Aider](https://aider.chat/docs/) and other coding agents** | Reasoning about code, deciding changes, editing across a project, running tests and integrating with Git | Not a replacement. Threadmoth is the deterministic mutation layer such agents can choose to use |

**Where Threadmoth has a genuine advantage:** when an automated edit must be *precise, bounded, minimally invasive, auditable and repeatable across file types*. Its superpower is the **shared editing contract**: one careful place to validate and commit changes proposed in several different ways, with a clear refusal when the evidence isn't there.

**Where another tool may be better:** whole-project semantic refactors, generated code, complex data transformations, interactive editing, formatting, compilation, testing and Git history management.

This comparison describes design goals and documented behaviour, **not a measured claim that Threadmoth is faster or more reliable than every alternative**. Reproducible head-to-head agent studies are still needed.

## Capabilities

Threadmoth's Target Registry classifies files by the level of understanding and preservation it can actually provide.

| Level | Coverage |
| --- | --- |
| **Structured** | JSON, JSONC, TOML, YAML, INI-style files, dotenv |
| **Syntax-aware** | JavaScript, TypeScript, JSX/TSX, Python, Rust, Go, C, C++, Java, C#, PHP, Bash, PowerShell, SQL and HCL |
| **Web/markup syntax** | HTML, CSS, XML |
| **Bounded document regions** | Markdown headings, lists and fenced regions |
| **Exact** | Explicit text replacements, strict patches, bounded patterns and desired-state operations for suitable UTF-8 files |
| **Structural (Toolbelt)** | AST pattern matching and rewriting across code trees via non-mutating specialist attachments (`ast-grep`) |
| **Opaque/refused** | Unsupported encodings, binary-like content or operations whose preservation guarantees cannot be established |

Support for a language means **syntax-aware locating and validation**, not full compiler understanding or semantic refactoring. HCL coverage is syntax-aware, not Terraform state or provider semantics. YAML and other formats deliberately refuse some constructs.

Other supported operations include guarded file creation, deletion, renaming and moving; bounded pre/postconditions; and ordered multi-file **content** transactions with journaling and recovery reporting.

**Current boundary:** file lifecycle operations are not composed with multi-file content replacements into one native atomic transaction. A rename plus reference rewrites is therefore **not** one guaranteed atomic Threadmoth operation today. Do not assume it is.

Find the installed capabilities rather than memorising this table:

~~~bash
threadmoth orient
threadmoth capabilities
threadmoth capabilities --for PATH --json
threadmoth inspect PATH --outline
threadmoth suggest PATH
threadmoth schema
~~~

See [Coverage](docs/coverage.md), [Target Registry](docs/target-registry.md), [CLI](docs/cli.md), [Edit documents](docs/edit-documents.md), [Protocol](docs/protocol.md) and [Architecture](docs/architecture.md).

## Made for independent agents

Threadmoth has **no embedded language model, subscription, model router or token meter**. It does not plan a feature, judge the quality of code or take control of the agent.

The agent can inspect a file, discover an operation, try Threadmoth, read a certificate and continue. If Threadmoth lacks a suitable operation, the agent may choose another **authorised** tool. A tool's limitation is not a universal permission boundary; a real workspace or security boundary still applies across all tools.

This is an AI-first design choice: **make ordinary software observable and self-explanatory to capable agents, rather than forcing agents through special rails.**

For CLI users, the ordinary operation is `threadmoth edit`. For MCP clients:

~~~bash
threadmoth mcp
~~~

Example MCP stdio configuration (the client must launch it with the intended checkout as its working directory):

~~~json
{
  "mcpServers": {
    "threadmoth": {
      "command": "threadmoth",
      "args": ["mcp"]
    }
  }
}
~~~

MCP and CLI share the same mutation engine. The server binds its workspace root **when the process starts**; an MCP request cannot silently rebind that authority. Restart a client after changing its binary or workspace configuration.

See [Agent integration](docs/agent-integration.md) and [OpenAI/Codex integration](docs/openai-integration.md).

## Install

Download a platform archive from the **[latest published GitHub release](https://github.com/matthewjameswatkins1978-cyber/Threadmoth/releases/latest)** and put the extracted `threadmoth` executable on `PATH`.

Published release archives have SHA-256 files and a release manifest. Check the archive against the release's checksum before installing. Windows and Linux have portable x86-64 builds and optional x86-64-v3 builds; macOS has Apple Silicon and x86-64 packages. Prefer portable unless you know the newer CPU baseline is supported.

~~~bash
threadmoth --version
threadmoth doctor --json
threadmoth capabilities
~~~

**The application version and wire protocol are separate.** Threadmoth uses application version `1.21.0` and protocol `1.4.0`. Repository code can be newer than the latest *published* release; the release page is the authority for downloads.

Build from source with Rust:

~~~bash
git clone https://github.com/matthewjameswatkins1978-cyber/Threadmoth.git
cd Threadmoth
cargo build --locked --release
~~~

Update an existing standalone installation explicitly:

~~~bash
threadmoth update --check
threadmoth update
~~~

Mutation is local. The explicit updater is the normal network-using exception. Do not overwrite an executable used by an active MCP process; identify its path and restart the client safely before relying on a changed installation.

## Testing and limits

Threadmoth ships unit and integration tests, schema/release checks and built-in adversarial benchmark profiles:

~~~bash
cargo test --locked
threadmoth benchmark --quick
threadmoth benchmark --tough
threadmoth benchmark --torture
~~~

Benchmarks report their own correctness and refusal outcomes, including `wrong_applied`. Previous local runs have reported `wrong_applied: 0`; that is **observed test evidence, not a universal proof of zero bugs or a head-to-head competitor victory**. See [Performance results](docs/performance-results.md).

Threadmoth's cooperative lock is not an OS-wide ban on external writes, and even journalled transaction recovery can require manual attention after exceptional failure. Binary formats and unsupported source constructs can refuse. **Exactness is more important than pretending every request is supported.**

## The contract

~~~text
If the requested change is exact, within scope and still valid:
    apply only the authorised edit
    read back and verify what landed
    return the evidence

Otherwise:
    refuse clearly, or report failure and recovery state
~~~

**Let the agent be clever. Let the edit be certain where it can be proven.**

**Find the exact thing. Change only that thing. Prove what happened.**

MIT licensed. Built in Rust. Model-agnostic. A careful pair of hands for the last mile of agentic software development.
