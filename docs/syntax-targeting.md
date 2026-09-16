# Syntax targeting

Threadmoth 1.10.0 uses Tree-sitter to identify source-node boundaries and validate candidate syntax. It does not unparse, pretty-print, serialize or regenerate an AST.

## AST-grounded

AST-grounded targeting guarantees exact source text, a syntax-node boundary and the required cardinality. The target may be structurally real without having a caller-requested grammatical role. Identical text in a string or comment is not silently treated as the same code target.

## AST-typed

AST-typed targeting additionally requires the caller to provide `node_kind`. It guarantees exact source text, a syntax-node boundary, the exact requested node kind and the required cardinality. Committed requests do not infer or strengthen `node_kind`; discovery may suggest a kind, but the caller must choose it explicitly.

## Current 1.9 syntax registry

The shared syntax engine has statically compiled grammars for:

```text
JavaScript
JSX
TypeScript
TSX
Python
Rust
Go
C
C++
Java
C#
PHP
HCL / Terraform syntax
Bash / Shell
PowerShell
SQL
HTML
CSS
XML
```

JSX and TSX are syntax variants. HTML, CSS and XML are web/markup targets, not programming-language count padding.

PHP grammar support includes PHP source and embedded PHP syntax. HCL support is syntax-level only and does not imply Terraform resource, provider, state or dependency semantics. SQL remains a common-dialect envelope rather than a promise of complete vendor-specific grammar coverage.

The web provider is deliberately structural: HTML, CSS and XML do not claim DOM semantics or deep embedded JavaScript/CSS semantic editing.

## Admission and fallback

A grammar is not admitted merely because a Tree-sitter package exists. Threadmoth expects deterministic parser integration, current-enough syntax coverage, acceptable maintenance/licensing, predictable malformed-input behaviour and adversarial tests.

Unsupported languages can still be edited through explicit exact routes when the file is valid text. A failed syntax request never silently becomes a text, regex, patch or desired-state request.

Runtime loading of arbitrary third-party grammars is intentionally unsupported. The compiled registry keeps the mutation surface deterministic and testable.
