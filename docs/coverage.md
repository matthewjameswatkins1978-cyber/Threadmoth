# Coverage model

Threadmoth 1.10.0 classifies a discovered file before mutation. The Target
Registry is the single classification authority used by discovery, inspect,
capabilities, suggest, CLI shorthands and MCP shorthands. The level is an
honest description of what the selected provider can prove, not a promise to
understand an entire ecosystem or rewrite a whole file.

| Level | Meaning | Current examples |
| --- | --- | --- |
| `structured` | Addressable keys, values, sections or sequences with source-local edits | JSON, JSONC, TOML, YAML, INI, dotenv |
| `syntax` | Parser-grounded source nodes; Core still owns every byte written | JavaScript, TypeScript, Python, Rust, Go, C, C++, Java, C#, PHP, Bash, PowerShell, SQL, HCL, HTML, CSS, XML, plus JSX/TSX variants |
| `region` | Bounded document regions without a full semantic model | Markdown |
| `exact` | No structural claim; exact text, strict patch, bounded pattern or desired-state routes remain explicit | unknown valid UTF-8, Dockerfile, Makefile, Java `.properties` |
| `opaque` | Unsupported encoding or binary-like input; text providers refuse | invalid UTF-8 or NUL-containing binary-like content |

Discovery exposes `target_kind`, provider, detection basis, confidence class, alternatives, `understanding_level`, `preservation_level` and explicit fallback routes.

A structured request never silently becomes an exact, regex, patch or desired-state request. Threadmoth may report weaker available routes, but the caller must explicitly choose one.

## Structured formats

### JSON / JSONC

JSON uses strict source-range structural edits. JSONC uses the JSON structural operation family while preserving supported comments/trailing-comma source layout. Neither provider needs to reserialize the whole document for a local edit.

### TOML

TOML uses structure-aware dotted-key targeting and narrows candidate changes to bounded source ranges. Representation drift outside the authorised range is a refusal.

### YAML

The 1.9 YAML provider supports useful nested paths through mappings and sequence indexes while keeping conservative source-preservation rules. It targets local scalar values and supports set, ensure-present, delete and ensure-absent where locality is provable.

Anchors, aliases, explicit tags, merge keys and directives remain outside the local preservation envelope and fail closed. Flow-style destructive edits and non-scalar targets can also refuse when Threadmoth cannot prove a safe local range. Multi-document boundaries are not guessed across.

### INI-style configuration

The INI provider understands source-level sections and keys for `.ini` files
and registered INI-style filenames such as `setup.cfg`, `tox.ini`, `pytest.ini`
and `.editorconfig`. It preserves comments, ordering and surrounding layout
where the local operation permits. Registry recognition does not imply that
every operation is valid for every target; unsupported operations refuse.

Threadmoth does not pretend every INI dialect has identical runtime semantics. It targets source structure only.

### dotenv

The dotenv provider remains a narrow key/value editor for `.env` filename
families and explicit `.env` extensions, preserving unrelated lines and
comments. The value shorthand reaches it through the same registry resolver as
INI and other structured providers.

## Syntax-aware source

Programming and declarative syntax targets in 1.10.0 are:

- JavaScript and TypeScript
- Python
- Rust
- Go
- C and C++
- Java
- C#
- PHP
- Bash / Shell
- PowerShell
- SQL common-dialect envelope
- HCL / Terraform syntax

JSX and TSX are supported syntax variants rather than separate language claims. HTML, CSS and XML use the same parser-grounded mechanism through the web provider.

PHP includes PHP source and embedded PHP parsing. HCL support does not include Terraform provider/resource/state semantics. HTML does not claim deep JavaScript/CSS semantics inside embedded script/style regions.

## Exact fallback and deliberate deferrals

Unknown valid UTF-8 can still use explicit exact mutation routes. Dockerfile and Makefile remain exact-text targets rather than overstated partial syntax support. Java `.properties` remains exact text because escaping, continuation and historical encoding rules do not fit Threadmoth's current UTF-8 source-preserving structured contract cleanly.

Kotlin, Swift and GNU Make syntax remain deferred until a parser/integration clears the same admission bar as existing syntax targets. Runtime loading of arbitrary third-party grammars remains intentionally unsupported.

The preservation levels exposed by discovery are `unrelated_bytes`, `bounded_region`, `explicit_desired_state` and `unavailable`.

> Parsers locate and validate. Threadmoth Core authorizes, applies and certifies candidate bytes.
