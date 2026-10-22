# Threadmoth Toolbelt Lab

A separate, black-box evaluation area for the Threadmoth CLI. The runner uses only Python's standard library; each case gets a temporary disposable workspace and is checked against exact expected bytes after the public CLI returns. The runner records SHA-256 hashes, exit/outcome, elapsed time, invocation count, and the full JSON certificate.

## Run

From the repository root:

```powershell
python lab/run.py --executable threadmoth --report lab/report.json
```

Pass an absolute executable path to evaluate another build. `--keep-workspaces` retains fixtures for diagnosis. A refusal is a passing case only when the expected outcome is `REFUSED` and the original bytes remain exact. Process success alone never passes a case.

The initial 14-case fixture set covers exact replacement, duplicate ambiguity, UTF-8 multibyte text, CRLF, a 140 KB distant edit, JSON source-preserving set, syntax-node replacement, 64 ordered replacements, a 200-replacement limit refusal, 2-file and 20-file transactions, stale pre-image, binary refusal, and dry-run preview. The 200-edit request is refused with `RESOURCE_LIMIT_EXCEEDED` (`source_text_edits` limit 64) and leaves the source unchanged; breaking a large rewrite into separate calls remains a coordination cost. Cases are declared as structured Python data in `run.py` so they can generate variable-size fixtures without checked-in bulk files.

The runner does not execute project hooks or test code. It runs only the supplied Threadmoth executable against lab-generated files.

## Baseline identity

The initial evaluation used canonical `main`, commit `502fd3b097b53163b2212cb648b61036c88b610c`, Threadmoth 1.20.0, protocol 1.4.0, Windows x86-64, and Rust 1.99.1. `baseline-lab.json` is the machine-readable report. Fresh `cargo test --locked` passed 292 tests across 23 test binaries. Built-in quick and tough benchmarks passed with `wrong_applied: 0`; torture passed 9 checks, skipped symlink creation because the host could not create one, and reported FOOTGUN-100 safe 100/100.

These results establish one Windows baseline, not cross-platform behaviour or AI-agent task quality. The built-in benchmark and torture suites remain the broader baseline corpus; this lab is an independent public-interface sample.

## Follow-up

See [structural acceptance](structural-acceptance.md) for ast-grep attachment gates and [semantic LSP feasibility](semantic-lsp.md) for the next adapter recommendation. Pending future capabilities remain acceptance scenarios; they do not fail the current baseline.