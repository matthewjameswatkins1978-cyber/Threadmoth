# Structural attachment acceptance

The current baseline already advertises code and web syntax-node operations backed by Tree-sitter. The separate ast-grep attachment must be measured as an observable provider/engine integration, not credited for that existing baseline. The host has ast-grep 0.45.3 installed; this is not evidence that Threadmoth invokes it. No attachment was found in the clean `main` checkout at `502fd3b097b53163b2212cb648b61036c88b610c`.

Run these against the attachment through its published CLI/MCP surface once available. First capture `--version`, capability discovery, and the exact attachment revision. Keep every fixture disposable and compare all resulting bytes independently.

| Gate | Fixture and operation | Required evidence |
| --- | --- | --- |
| One match | Rename a Rust function with a same-shaped unrelated function present | Only the selected symbol and intended references change |
| Repeated matches | Transform the same call shape in 30 files | Exact match/file counts; no unrelated edits |
| Multi-file | Change a public interface and imports/callers | Preview lists complete file set; application lands all expected files |
| Preview | Inspect a valid transform | Hashes before and after preview are identical |
| Ambiguity | Two candidate declarations fit without a caller choice | Explicit refusal or candidate selection; no first-match guess |
| Unsupported | Malformed source and unsupported grammar | Honest unsupported/parse result; unchanged bytes |
| Byte offsets | Put multibyte UTF-8 text before target; include CRLF and no final newline variants | Correct target bytes and exact untouched-byte identity |
| Stale/concurrent | Change a target after preview and before apply | Stale proposal refuses; no partial write |
| Trust boundary | Untrusted parser/rule configuration and an escaping symlink | Configuration is treated as data; workspace escape is refused |
| Failure | Cause one member of a multi-file transform to fail validation | No partial commit, or documented recoverable transaction with exact receipt |

Compare an equivalent transformation through the baseline Tree-sitter provider and ast-grep where operations overlap. Report support gaps separately from failures. Do not require a specific API shape before observing the attachment.