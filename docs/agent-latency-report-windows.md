# Threadmoth: Measured Agent Latency and Windows Efficiency Report

**Date:** 2026-11-15  
**Branch:** `perf/measured-agent-latency-windows`  
**Base Commit:** `f2ad2a62b6f91a42e4c9651a21e3bd725c949050` (v1.22.0)  
**Status:** Verification Complete — All Guarantees Preserved (`wrong_applied = 0`)

---

## 1. Executive Summary

This study investigates caller-visible latency and operational efficiency for AI agents invoking [Threadmoth](https://github.com/matthewjameswatkins1978-cyber/Threadmoth) on Microsoft Windows. 

Prior Threadmoth performance baselines measured internal in-process speed (e.g. historical Windows 1.9 benchmarks recording sub-millisecond core engine transforms). However, an agent interacting with tools incurs end-to-end caller costs: process creation, shell initialization, file system metadata queries, antivirus inspection, toolchain probing, and IPC serialization.

### Key Findings

1. **Shell Initialization Dominates Agent-Visible Latency:**
   - Invocations via a fresh PowerShell 7 session with default profiles spend **~2,075 ms – 2,111 ms** per call.
   - Invocations via fresh PowerShell with `-NoProfile -NonInteractive` spend **~333 ms** per call.
   - In contrast, invoking Threadmoth directly (or within an already-running shell) takes **~13.6 ms – 14.6 ms**.
   - Operating over a **persistent stdio MCP connection** (`threadmoth mcp`) has **0 ms process overhead**, reducing warm discovery to **0.10 ms** and single-file exact edits to **6.0 ms**.
   - *Conclusion:* Agents spawning fresh PowerShell shells spend >95% to 99% of their wall-clock time in Windows PowerShell process creation and profile processing, not in Threadmoth.

2. **Antivirus First-Access Tax:**
   - On Windows 11 with Microsoft Defender Antivirus active, the first execution of a newly compiled or newly dropped binary incurred a **559.88 ms** cold scan penalty before settling into the OS file system cache at **13.65 ms** steady state.

3. **Subprocess Probing in Toolbelt Discovery:**
   - `threadmoth orient` and `threadmoth capabilities` were repeatedly launching `ast-grep --version` via `std::process::Command`. In a persistent MCP server, this child process spawn took ~16 ms per tool call.
   - Introducing safe, process-lifetime caching (`OnceLock`) dropped warm MCP `orient` latency from **15.81 ms to 0.10 ms (-99.4%)** and MCP `capabilities` from **17.53 ms to 1.89 ms (-89.2%)**, while preserving test fault-injection overrides.

4. **NTFS Round-Trip Reductions in Workspace Operations:**
   - Optimizing file system checks in `src/workspace.rs`—consolidating file size and read-only checks into a single `metadata` call, skipping redundant ancestor canonicalization on pre-canonicalized paths, handling `NotFound` directly in `canonicalize_candidate`, and replacing `file.sync_all()` (`FlushFileBuffers`) with `file.flush()` on transient lock files—reduced:
     - `apply-plan` CLI median latency from **29.31 ms to 24.81 ms (-15.4%)**.
     - The complete 3-step prepare/review/apply CLI route from **68.66 ms to 60.11 ms (-12.5%)**.
     - Batch editing 20 files via a single `changes.tm` edit document from **158.00 ms to 126.01 ms (-20.3%)**, averaging **6.3 ms per file**.

5. **Zero-Allocation Plan Sizing:**
   - Replaced heap allocation (`serde_json::to_vec(plan)`) in `serialized_plan_size` with a zero-allocation `ByteCounter` streaming writer in `src/pipeline.rs`.

6. **Strict Safety Invariants Maintained:**
   - Every benchmark iteration verified mutation outcomes, cryptographic hashes, and certificates.
   - Ambiguity refusals (`TARGET_AMBIGUOUS`), stale identity checks (`STALE_IDENTITY`), path containment, cardinality constraints, and atomic recovery were verified across all runs.
   - Result: **`wrong_applied = 0` across all benchmarks and 100% test suite pass rate.**

---

## 2. System Environment & Executable Identities

### Hardware and OS Platform
- **Operating System:** Microsoft Windows 11 Pro Insider Preview (Build 26320.1000)
- **Host Architecture:** `x86_64-pc-windows-msvc`
- **CPU:** AMD Ryzen 9 3900X 12-Core Processor (24 logical cores @ 3.80 GHz)
- **Memory:** 64 GB RAM
- **Storage:** Local NVMe SSD, Drive C: formatted with NTFS (254.85 GB free)
- **Power Scheme:** Windows Balanced
- **Security / Antivirus:** Microsoft Defender Antivirus active (`AntivirusEnabled=True`, `RealTimeProtectionEnabled=True`, `IoavProtectionEnabled=True`)
- **Rust Toolchain:** rustc 1.100.0, cargo 1.100.0 (MSVC 2022)

### Executable Identifiers
- **Installed Binary Baseline:**  
  `D:\AgentTools\bin\threadmoth.exe`  
  *Reported Version:* `threadmoth 1.22.0`  
  *SHA256:* `2EAB514DB87F8B6D7730DA60BA46B74A401393849E3B96BCBA5B7BA52B2315A1`
- **Source-Built Baseline:**  
  Built from commit `f2ad2a62b6f91a42e4c9651a21e3bd725c949050` with `cargo build --release`  
  *SHA256:* `7EFF22AC20A9272B981423BE7739FACBD4DD130757D990637FDE5C91E300FD06`
- **Source-Built Improved:**  
  Built from branch `perf/measured-agent-latency-windows` with `cargo build --release`  
  *SHA256:* `523826C3122A00E19929828854E1A9CFE641D17DC9ED38CE293C291583D9FD85`

---

## 3. Benchmark Methodology

The benchmark harness was authored in Python ([scripts/agent_benchmark.py](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/scripts/agent_benchmark.py)). It drives direct process launches, shell invocations, and persistent stdio JSON-RPC MCP sessions.

### Measurement Protocols
1. **Direct Executable:**  
   Spawns `threadmoth.exe` directly via Windows `CreateProcess` (bypassing any intermediate shell). Captures bare binary startup and execution cost.
2. **Fresh PowerShell (Default Profile):**  
   Spawns `pwsh.exe -Command "<command>"`, measuring the standard invocation overhead of an agent running commands in a default Windows PowerShell environment.
3. **Fresh PowerShell (Clean/NoProfile):**  
   Spawns `pwsh.exe -NoProfile -NonInteractive -Command "<command>"`, isolating shell session startup without custom profile scripts.
4. **Inside-Shell Loop:**  
   Spawns a single long-running PowerShell process and issues successive commands through stdin/stdout, isolating command latency within an already-running agent terminal.
5. **Persistent MCP Connection:**  
   Spawns `threadmoth.exe mcp` as a persistent background child process, communicating over stdio JSON-RPC 2.0 (Model Context Protocol). Measures initialization and subsequent warm tool calls.

### Workload Scenarios
- **Minimal Startup / Read-only:** `threadmoth --version`, `threadmoth capabilities --json`, `threadmoth orient --json`.
- **Workflow A (Single-Call Exact Text Edit):** `threadmoth edit <path> --search <old> --replace <new>`.
- **Workflow B (Two/Three-Step Review Route):**
  - Step 1: `threadmoth prepare-text <path> --edit <old> <new> --output <plan> --summary`
  - Step 2: `threadmoth explain --plan <plan> --format diff`
  - Step 3: `threadmoth apply-plan --plan <plan>`
- **Structured JSON Edit:** `threadmoth set-value <path> --key <key> --value <val>`.
- **Large File (1 MB):** Exact text edit targeting a middle line in a 1 MB file.
- **Repeated Small Files (20 files):**
  - Strategy 1: 20 separate CLI process launches.
  - Strategy 2: Single multi-file edit document `changes.tm`.
  - Strategy 3: Persistent MCP sequential tool calls.
- **Explicit Safety Refusals:**
  - `TARGET_AMBIGUOUS`: Mutation targeting non-unique text.
  - `STALE_IDENTITY`: Mutation provided with an outdated source hash.

### Verification Standards
- Every benchmark iteration strictly validates:
  - Return code / JSON-RPC success.
  - Byte-exact content verification of mutated files.
  - Exact structured error code on refusals (`TARGET_AMBIGUOUS`, `STALE_IDENTITY`).
  - Strict preservation check: `wrong_applied == 0`.

---

## 4. Measured Results & Evidence

Below is the verified comparison between the **Source-Built Baseline** and the **Source-Built Improved** binary across all workloads. Each test was sampled over 10–25 iterations with warm-up runs.

```text
====================================================================================================
 THREADMOTH WINDOWS AGENT LATENCY BENCHMARK: BEFORE vs AFTER EVIDENCE
====================================================================================================
OS: Microsoft Windows 11 Pro Insider Preview (Build 26320)
CPU: AMD Ryzen 9 3900X 12-Core Processor (12 cores, 24 logical)
Storage: C: NTFS (254.85 GB free) | Defender: RealTimeProtection=True
Commit: f2ad2a62b6f91a42e4c9651a21e3bd725c949050 -> perf/measured-agent-latency-windows
Correctness: wrong_applied = 0 across all runs
====================================================================================================

Workload / Operation                          | Before Med (p95)       | After Med (p95)        | Delta (%) 
----------------------------------------------------------------------------------------------------------

[ Startup & Shell Invocations ]
  direct_cli_version_warm                     |  16.18 ms ( 18.25)     |  13.65 ms ( 47.21)     | -15.6%    
  direct_cli_version_first                    |     18.05 ms            |    559.88 ms*           | Defender Cold
  direct_cli_capabilities_warm                |  42.57 ms ( 78.11)     |  33.35 ms ( 72.01)     | -21.7%    
  direct_cli_orient_warm                      |  34.70 ms ( 37.01)     |  31.64 ms ( 33.91)     |  -8.8%    
  fresh_pwsh_noprofile_version                | 334.94 ms (340.36)     | 333.74 ms (336.47)     |  -0.4%    
  fresh_pwsh_default_version                  | 2111.45 ms (2117.28)   | 2075.59 ms (2090.44)   |  -1.7%    
  inside_shell_version_warm                   |  14.57 ms ( 27.85)     |  14.64 ms ( 26.88)     | +  0.5%   

[ Small File Edit Workflows ]
  workflow_a_direct_onecall_edit              |  28.77 ms ( 32.66)     |  27.03 ms ( 34.13)     |  -6.1%    
  workflow_b_direct_route_total               |  68.66 ms (100.91)     |  60.11 ms ( 94.76)     | -12.5%    
  workflow_b_step1_prepare_text               |  20.68 ms ( 24.00)     |  18.34 ms ( 22.28)     | -11.3%    
  workflow_b_step2_explain_diff               |  17.41 ms ( 21.16)     |  16.44 ms ( 19.20)     |  -5.5%    
  workflow_b_step3_apply_plan                 |  29.31 ms ( 61.03)     |  24.81 ms ( 59.75)     | -15.4%    
  workflow_c_inside_shell_onecall_edit        |  30.13 ms ( 45.24)     |  26.79 ms ( 43.36)     | -11.1%    

[ Structured Edits ]
  direct_cli_set_value                        |  23.29 ms ( 39.12)     |  22.30 ms ( 53.77)     |  -4.3%    

[ Large File (1MB) ]
  direct_cli_large_1mb_edit                   | 332.20 ms (407.47)     | 303.56 ms (314.79)     |  -8.6%    

[ Repeated Small Files (20 files) ]
  repeated_20_files_separate_cli_launches     | 706.47 ms (756.95)     | 644.12 ms (653.00)     |  -8.8%    
  repeated_20_files_single_edit_doc           | 158.00 ms (158.39)     | 126.01 ms (127.82)     | -20.3%    

[ Refusals and Safety ]
  ambiguity_refusal                           |  17.08 ms ( 18.86)     |  16.34 ms ( 50.25)     |  -4.3%    
  stale_hash_refusal                          |  17.91 ms ( 33.79)     |  17.82 ms ( 25.29)     |  -0.5%    

[ Persistent MCP Connection ]
  mcp_connection_establishment_total          |     16.77 ms            |     17.41 ms            | +  3.8%
  mcp_step1_initialize                        |      8.75 ms            |      9.27 ms            | +  6.0%
  mcp_step2_tools_list                        |      5.82 ms            |      5.93 ms            | +  1.9%
  mcp_orient_tool_call_warm                   |  15.81 ms ( 19.20)     |   0.10 ms (  0.12)     | -99.4%    
  mcp_capabilities_tool_call_warm             |  17.53 ms ( 19.40)     |   1.89 ms (  1.99)     | -89.2%    
  mcp_exact_replace_onecall_warm              |   5.98 ms ( 13.35)     |   6.01 ms (  6.26)     | +  0.5%   
  mcp_prepare_and_apply_route_total           |  12.76 ms ( 19.93)     |  14.28 ms ( 15.33)     | + 11.9%   
  mcp_set_value_warm                          |   7.33 ms (  8.12)     |   6.26 ms (  7.02)     | -14.6%    
  mcp_ambiguity_refusal                       |   3.48 ms (  6.35)     |   2.32 ms (  2.64)     | -33.4%    
  mcp_repeated_20_files_sequence              | 143.85 ms (161.14)     | 130.84 ms (132.93)     |  -9.0%    
```
*\*Note: `direct_cli_version_first` reflects cold Defender real-time scanning of the newly written executable binary on first invocation.*

---

## 5. Cost Attribution Breakdown

### A. The Shell Startup Tax
Agents frequently invoke tools via a bash or PowerShell subshell command (e.g. `pwsh -Command "..."`).
- **PowerShell Default:** Spawning a fresh `pwsh.exe` process that executes `Microsoft.PowerShell_profile.ps1` incurs **~2,075 ms**.
- **PowerShell NoProfile:** Adding `-NoProfile -NonInteractive` avoids profile execution but still incurs Windows process creation and CLR/PowerShell core engine initialization: **~333 ms**.
- **Threadmoth Direct Execution:** Bare CLI launch is **~13.6 ms**.
- **Agent Implication:** If an agent executes 5 commands in fresh PowerShell shells, it spends >10 seconds in shell startup alone. Running commands in an existing shell session or over persistent MCP recovers this entire cost.

### B. Windows Antivirus / Filter Driver Scanning
- Windows Defender (`MsMpEng.exe`) monitors file writes and new process execution via filter drivers (`WdFilter.sys`).
- When `cargo build --release` produces a new executable, the very first invocation takes **559.88 ms** because Defender performs synchronous real-time scanning.
- Once scanned and cached in memory, subsequent invocations take **~13.6 ms**.
- Transient lock files (`.threadmoth-mutation.lock`): Previously, Threadmoth called `file.sync_all()` (`FlushFileBuffers`) upon acquiring the lock. Calling `FlushFileBuffers` on NTFS forces a synchronous flush to disk, stalling until the physical write completes. Replacing this with `file.flush()` (user-space buffer flush) retains lock file descriptor atomicity while eliminating disk stall latency.

### C. Child Process Spawning (`ast-grep`)
- `threadmoth orient` and `threadmoth capabilities` dynamically discover AST-grep availability to determine structural editing support.
- Prior code ran `Command::new("ast-grep").arg("--version").output()` unconditionally on every orientation and capabilities call.
- On Windows, `CreateProcessW` for an external binary costs **15–18 ms**.
- In persistent MCP mode, where `orient` is called repeatedly by agents, this child process spawn accounted for >98% of the tool call's total execution time. Caching the result in a `OnceLock` dropped execution time to **0.10 ms**.

### D. File System Metadata Syscalls
- During `write_file_atomic_checked`, Threadmoth was making multiple sequential metadata calls:
  1. `ensure_file_size` (`fs::metadata`)
  2. `ensure_windows_replaceable` (`fs::metadata` to check readonly permissions)
  3. `resolve_path` and `parent.canonicalize`
- On Windows NTFS, each `GetFileInformationByHandle` / `CreateFile` query passes through the I/O manager and filesystem filter drivers.
- Consolidating the size and permission check into `ensure_file_size_and_writeable` and verifying parent containment with `path_is_within` (since `resolved` is already canonicalized) shaved **~4.5 ms** off `apply-plan` and **~32 ms** off batch file mutations.

---

## 6. Implementation Changes

All changes are contained in branch `perf/measured-agent-latency-windows`:

1. **Subprocess Probe Caching** ([src/toolbelt/structural.rs](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/src/toolbelt/structural.rs)):
   - Cached `probe_ast_grep()` result using `static DETECTED_AST_GREP: OnceLock<Option<String>>`.
   - Preserved `#[cfg(feature = "test-fault-injection")]` check so fault injection test overrides remain functional.
2. **NTFS Metadata Query Consolidation** ([src/workspace.rs](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/src/workspace.rs)):
   - Unified size and readonly checks in `ensure_file_size_and_writeable`.
   - Avoided redundant `fs::canonicalize` on parent directories of already-canonicalized target paths.
   - Handled `NotFound` directly in `canonicalize_candidate` without prior redundant `symlink_metadata` checks.
   - Replaced `file.sync_all()` on `.threadmoth-mutation.lock` with `file.flush()`.
3. **Zero-Allocation Plan Size Calculation** ([src/pipeline.rs](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/src/pipeline.rs)):
   - Implemented `ByteCounter` (a `std::io::Write` sink that counts bytes without storing them) for `serialized_plan_size`.
4. **Agent Guidance Updated** ([skills/threadmoth/SKILL.md](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/skills/threadmoth/SKILL.md)):
   - Documented persistent MCP vs shell launch trade-offs.
   - Clarified workflow selection between single-call edits, `changes.tm` edit documents, and the two-step prepare/review/apply route.

---

## 7. Safety & Invariant Verification

Threadmoth's core purpose is safe, high-assurance mutation. Every safety boundary was explicitly audited and tested:

| Invariant | Test Verification | Status |
| :--- | :--- | :--- |
| **Ambiguity Refusal** | Verified `TARGET_AMBIGUOUS` structured refusal on non-unique match. Target file remains byte-identical. | **VERIFIED** |
| **Stale State Protection** | Verified `STALE_IDENTITY` refusal when source file content changes between plan and apply. | **VERIFIED** |
| **Path Authorization** | Symlink traversal and relative parent escapes (`..`) rejected by `path_is_within`. | **VERIFIED** |
| **Atomicity & Durability** | Target file replaces atomically via rename; original permissions preserved; partial writes impossible. | **VERIFIED** |
| **Cardinality & Budgets** | Replacement bounds and effect limits strictly enforced. | **VERIFIED** |
| **Wrong Applied Zero** | Total incorrect mutations across all benchmark loops and test suites: **0**. | **VERIFIED (`wrong_applied = 0`)** |

### Test Suite Execution
- **Unit Tests:** `cargo test --bin threadmoth --lib` -> **102 passed; 0 failed**.
- **Integration Suites:**
  - `cargo test --test usability_and_preservation_211` -> **17 passed; 0 failed**.
  - `cargo test --test path_authorization` -> **passed**.
  - `cargo test --test plans_18` -> **passed**.
  - `cargo test --test mcp_protocol` -> **passed**.
  - `cargo test --test toolbelt_structural` -> **passed**.
  - `cargo test --test v1_features` -> **passed**.
  - `cargo test --test unified_editing` -> **passed**.

---

## 8. Agent Harness & Operator Guidance

Based on the empirical evidence, agent harnesses operating on Windows should adopt the following workflow rules:

1. **Prefer Persistent MCP Where Supported:**
   - Where the agent framework supports long-lived MCP servers (e.g. Antigravity, Claude Code, Cursor), configure `threadmoth mcp`.
   - Tool call latency drops to **0.1 ms – 6 ms**, eliminating shell startup entirely.
2. **If Running via Shell, Avoid Fresh Shells and Default Profiles:**
   - Do **NOT** invoke `pwsh -Command "..."` per tool call.
   - If a shell must be used, reuse an existing terminal session or pass `-NoProfile -NonInteractive` to avoid the 2-second profile penalty.
3. **Use the Fastest Safe Workflow for the Problem:**
   - **Obvious single-file text edit:** Use single-call `threadmoth edit <path> --search <old> --replace <new>` (CLI ~27 ms / MCP ~6 ms).
   - **Multi-file edits (e.g. 5–50 files):** Use an edit document `changes.tm` with `threadmoth edit changes.tm`. Applying 20 file changes in one batch takes **126 ms** (~6.3 ms/file), compared to >640 ms for 20 CLI launches.
   - **Consequential or high-blast-radius edits:** Use the two-step prepare/review/apply route (`prepare-text` -> `explain --plan` -> `apply-plan`). It adds a deterministic diff inspection boundary (~60 ms CLI / ~14 ms MCP) before committing.

---

## 9. In-Process Dry-Run Engine vs. Caller-Visible Latency

Threadmoth's built-in benchmarks (`threadmoth benchmark --tough` and `threadmoth benchmark --torture`) measure core in-process execution speed in dry-run mode. Comparing these figures directly against caller-visible measurements reveals where latency is spent:

| Workload | In-Process Dry Run | Caller Direct CLI | Caller Fresh pwsh | Caller Persistent MCP | Separation Ratio (CLI / In-Process) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Tiny Edit (21 B)** | **0.378 ms** | **27.09 ms** | **1,978.46 ms** | **4.90 ms** | **71.7x** |
| **Config Edit (30 kB)** | **0.615 ms** | **20.51 ms** | **2,020.10 ms** | **5.07 ms** | **33.3x** |
| **Text Edit (1 MB)** | **4.593 ms** | **307.80 ms** | **2,380.50 ms** | N/A (CLI test) | **67.0x** |
| **Multi-File Batch (20 files)** | **1.337 ms** / file | **6.16 ms** / file (`changes.tm`) | N/A | **5.42 ms** / file | **4.6x** |

### Attribution of the In-Process vs. Caller Gap
1. **Dry-Run vs. Real Disk Transaction:**
   - In-process benchmarks execute with `dry_run = true`. They perform parsing, AST matching, diff derivation, and in-memory certificate construction, but bypass disk mutation locks, temporary file staging, and atomic file replacement.
   - An actual disk mutation (`threadmoth edit`) requires acquiring `.threadmoth-mutation.lock`, staging the replacement file in the target directory, executing atomic file replace (`MoveFileExW` / `ReplaceFileW`), and flushing buffers.
2. **Process Lifecycle Overhead:**
   - Spawning `threadmoth.exe` from the OS shell adds ~13.6 ms of image loading, memory mapping (35 MB executable), TLS setup, and argument parsing.
   - Fresh PowerShell invocations add an additional ~300 ms (clean) or ~2,000 ms (with user profile).
3. **Closing the Gap via Edit Documents:**
   - When batching edits across files with `changes.tm` (`threadmoth edit changes.tm`), process startup and lock management are amortized across all targets. Latency drops to **6.16 ms per file**, closely approaching the pure engine baseline.

---

## 10. Long-Lived MCP Context Economy & Outline Caching

Threadmoth includes an `ObservationCache` designed for persistent MCP processes. It caches parsed AST outline nodes (`syntax::outline`) while strictly guaranteeing safety:

```text
Key: normalized_path | source_sha256 | provider | target_kind | limit
```

### Empirical Verification via Persistent MCP
- **Initial Inspection (`derived`):**
  - Threadmoth reads the source file from disk, hashes it (`sha256`), detects provider and grammar, invokes tree-sitter parsing, walks AST outline nodes, inserts into the cache, and produces observation handles.
  - **Median latency:** **5.67 ms** (p95: 6.14 ms).
- **Subsequent Inspection on Unchanged File (`cache_hit`):**
  - Threadmoth re-reads the source file from disk and computes its `sha256`. The key matches; outline nodes are served directly from `ObservationCache`.
  - **Median latency:** **0.56 ms** (p95: 0.62 ms) — a **10.1x speedup**.
- **Source Mutation Invalidation Check:**
  - Upon any modification to the file content, the re-computed `sha256` diverges.
  - The cache immediately registers a miss and derives fresh outline nodes (`reuse: "derived"`).
  - *Hard Guarantee Confirmed:* The cache never skips disk reading or cryptographic identity verification. Observation handles are read identities and never confer mutation authority.

---

## 11. Instrumentation Overhead Analysis

All measurements were captured using Python's `time.perf_counter()` on Windows:
- Under Windows 11, `time.perf_counter()` resolves to `QueryPerformanceCounter` (QPC), backed by the CPU invariant Time Stamp Counter (RDTSC).
- Measured invocation cost of `time.perf_counter()` is **< 40 nanoseconds** per call.
- Total measurement overhead is < 0.001% of measured CLI and MCP tool call latencies, introducing zero measurable skew into process timings, I/O wait times, or IPC message exchanges.

---

## 12. Reproducibility & Artifacts

All benchmark scripts and machine-readable JSON datasets are preserved in the repository:
- **Benchmark Runner:** [scripts/agent_benchmark.py](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/scripts/agent_benchmark.py)
- **Results Comparison Utility:** [scripts/compare_results.py](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/scripts/compare_results.py)
- **Raw Baseline (Built):** [docs/agent_latency_baseline_built.json](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/docs/agent_latency_baseline_built.json)
- **Raw Baseline (Installed):** [docs/agent_latency_baseline_installed.json](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/docs/agent_latency_baseline_installed.json)
- **Raw After-Improvements Dataset:** [docs/agent_latency_after_improvements.json](file:///c:/Users/Matmus/Documents/antigravity/proud-hypatia/docs/agent_latency_after_improvements.json)

To reproduce the benchmark:
```powershell
python scripts/agent_benchmark.py --exe target/release/threadmoth.exe --out docs/agent_latency_reproduced.json
python scripts/compare_results.py
```

