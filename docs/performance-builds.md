# Performance builds

Threadmoth 1.10.0 defines several build flavours, but all of them compile the same Core and provider set. Only code generation differs. A faster binary does not get a weaker safety path or a smaller capability contract.

| Flavour | Intended use | CPU baseline |
| --- | --- | --- |
| `portable` | default distributed release | compiler/target default |
| `modern` | optional distributed Windows/Linux x86-64 build | `x86-64-v3` |
| `native` | local build only | `target-cpu=native` |
| PGO experiment | evidence-gated release experiment | measured profile |

Portable is recommended when machine capabilities are uncertain. Modern artifacts are explicitly labelled `-v3` because they require the x86-64-v3 feature baseline. Native is never published as a general download because it is tuned to the machine that compiled it.

## Cargo profiles

The ordinary release profile remains separate from the named `maxperf` profile.

`maxperf` currently uses:

```text
opt-level = 3
lto = fat
codegen-units = 1
incremental = false
debug = false
strip = symbols
```

This profile does not alter global Cargo configuration or ordinary development builds.

Rust optimization flags are not treated as magic incantations. `opt-level=3`, fat LTO and one codegen unit are benchmark candidates, not proof of faster execution on every Threadmoth workload.

## Build scripts

Use:

```text
scripts/build-native.ps1
scripts/build-native.sh
scripts/build-modern.ps1
scripts/build-modern.sh
```

for explicit tuned local builds. The scripts scope `RUSTFLAGS` to their child build, use `--locked`, print useful compiler/output information, and embed build metadata that can be inspected with `threadmoth doctor --json`.

Modern x86 builds use `-C target-cpu=x86-64-v3`. Native uses `-C target-cpu=native` and must remain local-only.

## Release artifacts

The 1.9 release workflow is wired for portable Windows x86-64, Linux x86-64, macOS Apple Silicon and macOS x86-64 artifacts, plus explicit modern Windows/Linux x86-64-v3 artifacts.

The updater must preserve compatible flavour selection and verify the expected release manifest, archive/checksum asset and executable version. A portable installation must not silently become a CPU-specific build.

The v1.9.1 consistency release published these artifacts without changing the
build flavours; see the [GitHub Releases page](https://github.com/matthewjameswatkins1978-cyber/Threadmoth/releases/tag/v1.9.1) for the current artifact set.

## How optimization is selected

Optimization settings must be selected using measured Threadmoth workloads, including:

- CLI startup-heavy work;
- exact text and large-file edits;
- structured formats;
- syntax parsers;
- plans and apply-plan;
- transactions;
- common refusal paths.

Every candidate build must preserve the correctness contract and report zero wrong successful mutations.

The current local smoke data is recorded in [Performance results](performance-results.md). It shows that different configurations win different cases, so no configuration is labelled the universal winner yet.

The full opt-level 2/3 and thin/fat LTO matrix and separate PGO training/validation remain follow-up performance work. They are not required to publish the portable and modern 1.9.0 artifacts, but remain prerequisites for declaring a final performance winner or shipping PGO. Final cross-platform runtime verification and the published-release updater exercise are release-acceptance gates. PGO is not enabled by default.
