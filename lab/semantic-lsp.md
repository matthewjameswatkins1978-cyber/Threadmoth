# Semantic editing feasibility

## Recommendation

Prototype **rust-analyzer first**, behind a narrow proposal adapter that returns a Threadmoth-owned guarded plan. Rust is the primary Threadmoth implementation language, and rename/refactor is a bounded first operation. Keep the server as a proposal engine: do not let it write files or bypass Threadmoth's workspace identity, effect budgets, preview, commit and read-back verification.

Then test **Pyright** on Python cross-file rename and import-related edits. Treat **TypeScript** as third: its TypeScript Language Server is a thin LSP layer over `tsserver`, includes custom execute-command/refactor paths and can depend on workspace formatting configuration. Re-evaluate TypeScript's native LSP path as upstream packaging evolves.

## Observable protocol facts

LSP `textDocument/rename` returns `WorkspaceEdit | null`. The edit may use a URI-keyed `changes` map or ordered `documentChanges`; the latter can carry a document version and resource operations. Clients negotiate position encodings; UTF-16 is the default when no encoding is negotiated. A returned range is therefore not a UTF-8 byte offset.

Pyright's current server code returns `WorkspaceEdit` from its rename provider and keeps opened document versions/text in its service. The TypeScript Language Server implements rename by requesting `Rename` from `tsserver` and translating its returned locations; its public docs also describe a rename-file command and custom refactor commands. Neither a tool's advertised feature nor an LSP response establishes that a particular workspace operation is semantically correct.

## Integration boundary

1. Start one server for a disposable workspace; record binary/package version, startup time, initialize capabilities, and clean shutdown.
2. Open the source documents with explicit monotonically increasing versions; record each exact on-disk SHA-256 and URI-to-contained-path mapping.
3. Request prepare-rename, then rename at a known position. Preserve the raw JSON-RPC response and server logs.
4. Accept only supported `WorkspaceEdit` forms. Reject unknown resource operations, URI escapes, duplicate/overlapping ranges, unknown documents, and edits without a matching source identity.
5. Convert negotiated LSP positions to UTF-8 byte offsets using the exact source snapshot and encoding; test astral Unicode before edits, CRLF, and lines with tabs. Reject positions that cannot be mapped unambiguously.
6. If a versioned edit version differs from the opened snapshot, refuse. If the edit is unversioned, require the current file hash to equal the captured hash before constructing Threadmoth operations.
7. Submit the proposal through Threadmoth preview. Independently verify preview changes, complete path set, budgets, and hashes; mutate only after preview succeeds. Check landed bytes against a separately authored expected result.

The adapter must not resolve stale edits by searching for similar text. Do not let server-supplied paths expand authority. The first test set should cover local rename, cross-file rename, shadowed same-name symbols, unresolved imports, stale document versions, unversioned `changes`, emoji before a target, resource rename, cancelled request, server crash, and partial failure.

## Cost and licensing

The local Windows host had rust-analyzer 1.99.1 and ast-grep 0.45.3; Pyright and `typescript-language-server` were not on PATH. No servers were installed or launched for this feasibility pass, so startup/latency numbers remain **unmeasured**. Measure cold start, warm rename latency, and memory on the same small/mid/large fixture projects before choosing defaults.

rust-analyzer is distributed under MIT OR Apache-2.0. Pyright is MIT-licensed. typescript-language-server is MIT-licensed and depends on the TypeScript server implementation; record the selected package and TypeScript version with each run. Confirm transitive package licenses and redistribution obligations before bundling any engine. Prefer user-provided executables or documented opt-in installation for the initial adapter.

## Sources

- [LSP 3.18 rename request](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.18/language/rename.md)
- [LSP 3.17 WorkspaceEdit: versioned document changes and resource operations](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/types/workspaceEdit.md) and [position encoding definitions](https://github.com/microsoft/vscode-languageserver-node/blob/main/protocol/src/common/protocol.ts)
- [Pyright language server rename handler](https://github.com/microsoft/pyright/blob/main/packages/pyright-internal/src/languageServerBase.ts)
- [TypeScript Language Server implementation](https://github.com/typescript-language-server/typescript-language-server/blob/master/src/lsp-server.ts)
- [rust-analyzer repository and licensing](https://github.com/rust-lang/rust-analyzer)
- [Pyright repository and licensing](https://github.com/microsoft/pyright)
- [TypeScript Language Server repository and licensing](https://github.com/typescript-language-server/typescript-language-server)