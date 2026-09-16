# Target registry

`src/target_registry.rs` is the canonical classification authority in Threadmoth 1.10.0. Runtime discovery, inspect, capabilities, suggest, CLI shorthand provider selection and MCP shorthand provider selection all use it to answer a simple question before mutation:

> What does Threadmoth know about this file, and what level of mutation can it safely offer?

Each registered target can describe:

- canonical ID and display name;
- category;
- provider ownership;
- aliases;
- extensions;
- exact filenames;
- understanding level;
- preservation level;
- whether explicit weaker fallback routes are available.

Capability, inspect and suggest surfaces derive path classification from this same registry. The Tree-sitter grammar table supplies parser implementations for syntax targets but is not a second user-facing language list.

The shared shorthand resolver consumes the registry result and only then checks
whether the requested value operation is available for that provider. It does
not maintain a second extension or filename table. Consequently, `setup.cfg`,
`tox.ini` and `.editorconfig` resolve to the INI provider, `.env.local` resolves
to dotenv, and conventional extensions resolve in exactly the same way across
CLI and MCP.

## Categories

Current descriptive target categories include programming languages, declarative languages, syntax variants, structured formats, markup, documents and exact text. Categories are metadata, not separate mutation engines.

For example, JSX/TSX are syntax variants, HCL is declarative syntax, HTML/XML are markup, and Dockerfile/Makefile deliberately remain exact text.

## Deterministic detection

Detection follows evidence rather than registration order or fuzzy guessing. The current path includes:

1. reject invalid UTF-8 / binary-like NUL content as opaque when bytes are available;
2. recognize exact registered filenames;
3. recognize the `.env` filename family;
4. match a unique registered extension;
5. use recognized shebang evidence where applicable;
6. report ambiguous structured-looking content as ambiguous evidence rather than choosing a provider;
7. fall back to exact valid UTF-8 text;
8. otherwise remain opaque.

Recognized shebangs include common Python, PowerShell, Bash/sh and Node forms.

Content evidence is not permission. If content resembles several structured formats, Threadmoth reports alternatives and leaves provider choice explicit.

Registry tests reject duplicate canonical IDs, alias collisions and conflicting extension ownership. Adding a new syntax target also requires a matching grammar implementation and tests; a filename entry alone does not create structural support.

## Coverage and preservation

The 1.9 understanding ladder is:

```text
structured
syntax
region
exact
opaque
```

Preservation metadata uses:

```text
unrelated_bytes
bounded_region
explicit_desired_state
unavailable
```

Fallback routes are advisory capability metadata. Threadmoth never silently converts a structured or syntax operation into a weaker text/pattern/patch/desired-state mutation.

See [Coverage](coverage.md) for the user-facing support matrix and deliberate deferrals.
