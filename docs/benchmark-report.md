# Benchmark report

Threadmoth's benchmark suite is correctness-first. It measures deterministic local execution, not AI token savings or model quality.

The primary falsification metric is **wrong mutation reported as successful**. Every expected successful mutation has its resulting bytes checked; malformed, ambiguous and stale cases are expected to refuse. A timing result is not considered useful if correctness is wrong.

## Commands

```text
threadmoth benchmark --quick
threadmoth benchmark
threadmoth benchmark --tough
threadmoth benchmark --torture
```

Add `--json` for machine-readable output. The compatibility forms `threadmoth benchmark tough` and `threadmoth torture` remain accepted, but the flag forms are canonical.

The tough profile includes large text, long-line refinement, many-line input and repeated small-file work. Torture adds deterministic safety regressions, transaction/recovery checks, containment cases where the host supports them, and FOOTGUN-100.

## Retained 1.9 local performance evidence

The current checked-in [performance results](performance-results.md) record Windows x86-64 smoke measurements from Rust 1.98.1. Each build ran the built-in tough profile and every run reported `wrong_applied: 0`.

| Build | Size (bytes) | tiny (us) | config 30k (us) | text 1m (us) | text 5m (us) | text 32m (us) | 250 small files (us) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| portable release | 34,688,512 | 389.956 | 699.410 | 7,481.520 | 37,073.540 | 240,496.800 | 3,977.808 |
| portable maxperf | 33,084,416 | 475.708 | 711.678 | 7,947.524 | 40,352.530 | 258,230.033 | 1,246.616 |
| modern x86-64-v3 | 33,177,088 | 409.793 | 675.416 | 7,956.200 | 32,575.700 | 213,768.200 | 1,208.274 |
| native local | 33,056,768 | 411.670 | 702.258 | 6,535.620 | 32,555.510 | 210,113.667 | 1,292.970 |

These are local measurements, not cross-platform guarantees. They show why Threadmoth does not simply label one compiler profile "fastest": different workloads favour different configurations.

The 1.9 full opt-level 2/3 and thin/fat LTO matrix and separate PGO training/validation remain follow-up performance work, not publication blockers for the portable and modern artifacts. They remain prerequisites for declaring a final performance winner or shipping PGO. The published-artifact updater exercise remains part of release acceptance. See [Performance builds](performance-builds.md) for the policy.

## Binary-size context

The previously installed 1.8.1 Windows executable was 27,543,552 bytes. The measured 1.9 portable Windows binary was 34,688,512 bytes. The increase reflects the admitted Java, C#, PHP, HCL and YAML grammars plus the new coverage registry and INI provider.

That size increase is recorded rather than hidden. Threadmoth does not fragment the official capability set into a plugin zoo merely to make the executable look smaller.

## Reporting benchmark evidence

Record at least:

- Threadmoth version and commit;
- Rust and LLVM/toolchain version;
- OS, target triple and CPU;
- build flavour and compiler flags;
- binary size;
- benchmark profile;
- raw/summary timings;
- `wrong_applied` or equivalent correctness result.

The important release signal remains:

```text
wrong successful mutations = 0
```
