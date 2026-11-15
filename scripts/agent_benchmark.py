#!/usr/bin/env python3
"""
Threadmoth Agent Latency & Windows Efficiency Benchmark Harness

Measures end-to-end caller-visible latency across:
- Direct CLI execution (no shell)
- Fresh PowerShell (default profile)
- Fresh PowerShell (-NoProfile -NonInteractive)
- Inside an already-running shell session
- Persistent MCP connection (stdio JSON-RPC)

Validates correctness (wrong_applied = 0), certificates, and refusal behavior on every case.
Records raw timings, distributions (median, p95, min, max, spread), system facts,
and machine-readable summaries.
"""

import os
import sys
import time
import json
import shutil
import tempfile
import platform
import subprocess
import statistics
from pathlib import Path

def get_system_metadata(exe_path):
    metadata = {
        "os_caption": platform.platform(),
        "python_version": sys.version,
        "exe_path": os.path.abspath(exe_path),
        "exe_size": os.path.getsize(exe_path) if os.path.exists(exe_path) else None,
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime()),
    }
    # Get executable sha256
    if os.path.exists(exe_path):
        import hashlib
        h = hashlib.sha256()
        with open(exe_path, "rb") as f:
            while chunk := f.read(65536):
                h.update(chunk)
        metadata["exe_sha256"] = h.hexdigest()

    # Get git revision if available
    try:
        git_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        metadata["git_sha"] = git_sha
        git_branch = subprocess.check_output(["git", "rev-parse", "--abbrev-ref", "HEAD"], text=True).strip()
        metadata["git_branch"] = git_branch
    except Exception:
        pass

    # Query PowerShell for Windows-specific facts
    try:
        ps_cmd = """
        $os = Get-CimInstance Win32_OperatingSystem
        $cpu = Get-CimInstance Win32_Processor
        $volC = Get-Volume -DriveLetter C
        $def = Get-MpComputerStatus
        [PSCustomObject]@{
            os_name = $os.Caption
            os_version = $os.Version
            os_build = $os.BuildNumber
            cpu_name = $cpu.Name
            cpu_cores = $cpu.NumberOfCores
            cpu_logical = $cpu.NumberOfLogicalProcessors
            c_fs = $volC.FileSystemType
            c_free_gb = [math]::Round($volC.SizeRemaining / 1GB, 2)
            c_size_gb = [math]::Round($volC.Size / 1GB, 2)
            antivirus_enabled = $def.AntivirusEnabled
            rtp_enabled = $def.RealTimeProtectionEnabled
        } | ConvertTo-Json
        """
        out = subprocess.check_output(["pwsh.exe", "-NoProfile", "-NonInteractive", "-Command", ps_cmd], text=True)
        ps_facts = json.loads(out)
        metadata.update(ps_facts)
    except Exception as e:
        metadata["ps_facts_error"] = str(e)

    return metadata

def compute_stats(samples):
    if not samples:
        return {}
    sorted_s = sorted(samples)
    n = len(sorted_s)
    mean_val = statistics.mean(sorted_s)
    med_val = statistics.median(sorted_s)
    min_val = min(sorted_s)
    max_val = max(sorted_s)
    std_val = statistics.stdev(sorted_s) if n > 1 else 0.0
    # p95 calculation
    p95_idx = int(0.95 * (n - 1))
    p95_val = sorted_s[p95_idx]
    # IQR calculation
    q25_idx = int(0.25 * (n - 1))
    q75_idx = int(0.75 * (n - 1))
    iqr_val = sorted_s[q75_idx] - sorted_s[q25_idx]
    return {
        "count": n,
        "mean_ms": round(mean_val, 3),
        "median_ms": round(med_val, 3),
        "p95_ms": round(p95_val, 3),
        "min_ms": round(min_val, 3),
        "max_ms": round(max_val, 3),
        "std_ms": round(std_val, 3),
        "iqr_ms": round(iqr_val, 3),
        "raw_ms": [round(x, 3) for x in samples],
    }

class BenchmarkRunner:
    def __init__(self, exe_path):
        self.exe = os.path.abspath(exe_path)
        self.wrong_applied = 0
        self.results = {}

    def run_direct_cli(self, args, cwd=None, input_bytes=None):
        t0 = time.perf_counter()
        p = subprocess.run([self.exe] + args, cwd=cwd, input=input_bytes, capture_output=True)
        t1 = time.perf_counter()
        return (t1 - t0) * 1000.0, p.returncode, p.stdout, p.stderr

    def run_pwsh_fresh(self, cmd_str, no_profile=False, cwd=None):
        ps_args = ["pwsh.exe"]
        if no_profile:
            ps_args.extend(["-NoProfile", "-NonInteractive"])
        ps_args.extend(["-Command", cmd_str])
        t0 = time.perf_counter()
        p = subprocess.run(ps_args, cwd=cwd, capture_output=True)
        t1 = time.perf_counter()
        return (t1 - t0) * 1000.0, p.returncode, p.stdout, p.stderr

    # Benchmark: Startup & Read-only commands
    def bench_startup_suite(self, repetitions=15):
        suite = {}

        # 1. Direct CLI --version
        samples_first = []
        samples_warm = []
        # First-access run
        dur, code, out, err = self.run_direct_cli(["--version"])
        assert code == 0, f"version failed: {err}"
        samples_first.append(dur)
        for _ in range(repetitions):
            dur, code, out, err = self.run_direct_cli(["--version"])
            assert code == 0
            samples_warm.append(dur)
        suite["direct_cli_version_warm"] = compute_stats(samples_warm)
        suite["direct_cli_version_first"] = round(samples_first[0], 3)

        # 2. Direct CLI minimal read-only (capabilities --json)
        samples_caps = []
        for _ in range(repetitions):
            dur, code, out, err = self.run_direct_cli(["capabilities", "--json"])
            assert code == 0
            samples_caps.append(dur)
        suite["direct_cli_capabilities_warm"] = compute_stats(samples_caps)

        # 3. Direct CLI orient --json
        samples_orient = []
        for _ in range(repetitions):
            dur, code, out, err = self.run_direct_cli(["orient", "--json"])
            assert code == 0
            samples_orient.append(dur)
        suite["direct_cli_orient_warm"] = compute_stats(samples_orient)

        # 4. Fresh pwsh -NoProfile -NonInteractive --version (5 reps due to shell weight)
        samples_ps_noprofile = []
        cmd = f"& '{self.exe}' --version"
        for _ in range(7):
            dur, code, out, err = self.run_pwsh_fresh(cmd, no_profile=True)
            assert code == 0
            samples_ps_noprofile.append(dur)
        suite["fresh_pwsh_noprofile_version"] = compute_stats(samples_ps_noprofile)

        # 5. Fresh pwsh default profile --version (5 reps)
        samples_ps_default = []
        for _ in range(5):
            dur, code, out, err = self.run_pwsh_fresh(cmd, no_profile=False)
            assert code == 0
            samples_ps_default.append(dur)
        suite["fresh_pwsh_default_version"] = compute_stats(samples_ps_default)

        # 6. Inside existing shell: measure repeated invocations inside one long-lived PowerShell process
        ps_script = f"""
        $exe = '{self.exe}'
        $results = @()
        for ($i = 0; $i -lt {repetitions}; $i++) {{
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            & $exe --version | Out-Null
            $sw.Stop()
            $results += $sw.Elapsed.TotalMilliseconds
        }}
        $results | ConvertTo-Json
        """
        _, code, out, err = self.run_pwsh_fresh(ps_script, no_profile=True)
        assert code == 0
        samples_inside_shell = json.loads(out)
        suite["inside_shell_version_warm"] = compute_stats(samples_inside_shell)

        self.results["startup_and_shell"] = suite

    # Benchmark: Small file exact edit workflows
    def bench_small_edit_suite(self, repetitions=15):
        suite = {}
        with tempfile.TemporaryDirectory() as td:
            # Fixture: small 2 KB file with a unique target
            fixture_path = os.path.join(td, "config.ini")
            template = "[app]\nname=threadmoth\nversion=1.22.0\nenvironment=production\n" + ("comment=line\n" * 40) + "target_key=find_me_exact\n" + ("trailing=data\n" * 20)

            # --- Workflow A: Direct CLI One-Call Edit (`threadmoth edit`) ---
            samples_onecall = []
            for _ in range(repetitions):
                with open(fixture_path, "w", encoding="utf-8") as f:
                    f.write(template)
                dur, code, out, err = self.run_direct_cli(
                    ["edit", "config.ini", "--search", "target_key=find_me_exact", "--replace", "target_key=replaced_exact", "--json"],
                    cwd=td
                )
                with open(fixture_path, "r", encoding="utf-8") as f:
                    content = f.read()
                if code != 0 or "target_key=replaced_exact" not in content or "find_me_exact" in content:
                    self.wrong_applied += 1
                assert code == 0, f"onecall edit failed: {err.decode()}"
                cert = json.loads(out)
                assert cert.get("outcome") == "APPLIED"
                samples_onecall.append(dur)
            suite["workflow_a_direct_onecall_edit"] = compute_stats(samples_onecall)

            # --- Workflow B: Direct CLI Prepare/Review/Apply Route ---
            # Step 1: prepare-text -> plan.json
            # Step 2: explain --plan -> review diff
            # Step 3: apply-plan -> commit
            samples_route_total = []
            samples_route_prep = []
            samples_route_explain = []
            samples_route_apply = []
            plan_path = os.path.join(td, "plan.json")

            for _ in range(repetitions):
                with open(fixture_path, "w", encoding="utf-8") as f:
                    f.write(template)
                if os.path.exists(plan_path):
                    os.remove(plan_path)
                
                # Step 1: prepare-text
                t_prep_0 = time.perf_counter()
                dur_prep, code1, out1, err1 = self.run_direct_cli(
                    ["prepare-text", "config.ini", "--edit", "target_key=find_me_exact", "target_key=replaced_exact", "--output", plan_path, "--summary"],
                    cwd=td
                )
                t_prep_1 = time.perf_counter()
                assert code1 == 0, f"prepare-text failed: {err1.decode()}"
                prep_ms = (t_prep_1 - t_prep_0) * 1000.0

                # Step 2: explain diff
                t_exp_0 = time.perf_counter()
                dur_exp, code2, out2, err2 = self.run_direct_cli(
                    ["explain", "--plan", plan_path, "--format", "diff"],
                    cwd=td
                )
                t_exp_1 = time.perf_counter()
                assert code2 == 0, f"explain failed: {err2.decode()}"
                exp_ms = (t_exp_1 - t_exp_0) * 1000.0

                # Step 3: apply-plan
                t_app_0 = time.perf_counter()
                dur_app, code3, out3, err3 = self.run_direct_cli(
                    ["apply-plan", "--plan", plan_path],
                    cwd=td
                )
                t_app_1 = time.perf_counter()
                assert code3 == 0, f"apply-plan failed: {err3.decode()}"
                app_ms = (t_app_1 - t_app_0) * 1000.0

                with open(fixture_path, "r", encoding="utf-8") as f:
                    content = f.read()
                if "target_key=replaced_exact" not in content or "find_me_exact" in content:
                    self.wrong_applied += 1
                cert = json.loads(out3)
                assert cert.get("outcome") == "APPLIED"

                total_ms = prep_ms + exp_ms + app_ms
                samples_route_total.append(total_ms)
                samples_route_prep.append(prep_ms)
                samples_route_explain.append(exp_ms)
                samples_route_apply.append(app_ms)

            suite["workflow_b_direct_route_total"] = compute_stats(samples_route_total)
            suite["workflow_b_step1_prepare_text"] = compute_stats(samples_route_prep)
            suite["workflow_b_step2_explain_diff"] = compute_stats(samples_route_explain)
            suite["workflow_b_step3_apply_plan"] = compute_stats(samples_route_apply)

            # --- Workflow C: Inside Shell One-Call Edit ---
            ps_edit_script = f"""
            $exe = '{self.exe}'
            $results = @()
            $template = @'
{template}
'@
            for ($i = 0; $i -lt {repetitions}; $i++) {{
                Set-Content -Path 'config.ini' -Value $template -NoNewline
                $sw = [System.Diagnostics.Stopwatch]::StartNew()
                & $exe edit config.ini --search 'target_key=find_me_exact' --replace 'target_key=replaced_exact' --json | Out-Null
                $sw.Stop()
                $results += $sw.Elapsed.TotalMilliseconds
            }}
            $results | ConvertTo-Json
            """
            _, code, out, err = self.run_pwsh_fresh(ps_edit_script, no_profile=True, cwd=td)
            assert code == 0, f"inside shell edit failed: {err}"
            samples_inside_shell_edit = json.loads(out)
            suite["workflow_c_inside_shell_onecall_edit"] = compute_stats(samples_inside_shell_edit)

        self.results["small_file_workflows"] = suite

    # Benchmark: Structured edits (JSON set-value)
    def bench_structured_suite(self, repetitions=15):
        suite = {}
        with tempfile.TemporaryDirectory() as td:
            fixture_path = os.path.join(td, "package.json")
            template = json.dumps({"name": "test-pkg", "version": "1.0.0", "dependencies": {"dep_a": "1.0.0", "dep_b": "2.0.0"}}, indent=2)

            samples_set_value = []
            for _ in range(repetitions):
                with open(fixture_path, "w", encoding="utf-8") as f:
                    f.write(template)
                dur, code, out, err = self.run_direct_cli(
                    ["set-value", "package.json", "version", '"1.0.1"'],
                    cwd=td
                )
                assert code == 0, f"set-value failed: {err.decode()}"
                with open(fixture_path, "r", encoding="utf-8") as f:
                    data = json.load(f)
                if data.get("version") != "1.0.1":
                    self.wrong_applied += 1
                cert = json.loads(out)
                assert cert.get("outcome") == "APPLIED"
                samples_set_value.append(dur)
            suite["direct_cli_set_value"] = compute_stats(samples_set_value)

        self.results["structured_edits"] = suite

    # Benchmark: Larger file edit (1 MB text file)
    def bench_large_file_suite(self, repetitions=10):
        suite = {}
        with tempfile.TemporaryDirectory() as td:
            fixture_path = os.path.join(td, "large_source.txt")
            # 1 MB of text with a unique target in the middle
            chunk = "fn process_token_stream(id: u64, name: &str) -> Result<Token, Error> {\n    let valid = true;\n    Ok(Token::new(id))\n}\n"
            reps = 1_000_000 // len(chunk)
            content = (chunk * (reps // 2)) + "fn TARGET_FUNCTION_UNIQUE() -> bool { false }\n" + (chunk * (reps // 2))

            samples_large = []
            for _ in range(repetitions):
                with open(fixture_path, "w", encoding="utf-8") as f:
                    f.write(content)
                dur, code, out, err = self.run_direct_cli(
                    ["edit", "large_source.txt", "--search", "TARGET_FUNCTION_UNIQUE", "--replace", "REPLACED_FUNCTION_UNIQUE", "--json"],
                    cwd=td
                )
                assert code == 0, f"large edit failed: {err.decode()}"
                with open(fixture_path, "r", encoding="utf-8") as f:
                    res = f.read()
                if "REPLACED_FUNCTION_UNIQUE" not in res or "TARGET_FUNCTION_UNIQUE" in res:
                    self.wrong_applied += 1
                cert = json.loads(out)
                assert cert.get("outcome") == "APPLIED"
                samples_large.append(dur)
            suite["direct_cli_large_1mb_edit"] = compute_stats(samples_large)

        self.results["large_file"] = suite

    # Benchmark: Repeated small files (20 small files)
    def bench_repeated_small_files(self, count=20, repetitions=5):
        suite = {}
        with tempfile.TemporaryDirectory() as td:
            # Approach A: 20 separate direct CLI invocations
            samples_per_file_total = []
            for _ in range(repetitions):
                # Setup 20 files
                for i in range(count):
                    with open(os.path.join(td, f"file_{i}.txt"), "w", encoding="utf-8") as f:
                        f.write(f"prefix data\ntarget_token={i}\nsuffix data\n")
                t0 = time.perf_counter()
                for i in range(count):
                    dur, code, out, err = self.run_direct_cli(
                        ["edit", f"file_{i}.txt", "--search", f"target_token={i}", "--replace", f"target_token=done_{i}", "--json"],
                        cwd=td
                    )
                    assert code == 0
                t1 = time.perf_counter()
                # Verify
                for i in range(count):
                    with open(os.path.join(td, f"file_{i}.txt"), "r", encoding="utf-8") as f:
                        txt = f.read()
                    if f"target_token=done_{i}" not in txt:
                        self.wrong_applied += 1
                samples_per_file_total.append((t1 - t0) * 1000.0)
            suite["repeated_20_files_separate_cli_launches"] = compute_stats(samples_per_file_total)

            # Approach B: Single edit document (.tm) covering all 20 files
            samples_edit_doc = []
            doc_lines = []
            for i in range(count):
                doc_lines.append(f"*** File: file_{i}.txt\n<<<<<<< SEARCH\ntarget_token={i}\n=======\ntarget_token=done_{i}\n>>>>>>> REPLACE\n")
            doc_content = "\n".join(doc_lines)
            doc_path = os.path.join(td, "batch_changes.tm")
            with open(doc_path, "w", encoding="utf-8") as df:
                df.write(doc_content)

            for _ in range(repetitions):
                for i in range(count):
                    with open(os.path.join(td, f"file_{i}.txt"), "w", encoding="utf-8") as f:
                        f.write(f"prefix data\ntarget_token={i}\nsuffix data\n")
                dur, code, out, err = self.run_direct_cli(
                    ["edit", "batch_changes.tm", "--json"],
                    cwd=td
                )
                assert code == 0, f"batch edit failed: {err.decode()}"
                for i in range(count):
                    with open(os.path.join(td, f"file_{i}.txt"), "r", encoding="utf-8") as f:
                        txt = f.read()
                    if f"target_token=done_{i}" not in txt:
                        self.wrong_applied += 1
                samples_edit_doc.append(dur)
            suite["repeated_20_files_single_edit_doc"] = compute_stats(samples_edit_doc)

        self.results["repeated_small_files"] = suite

    # Benchmark: Expected Refusals (Ambiguity & Stale pre-hash)
    def bench_refusals_suite(self, repetitions=15):
        suite = {}
        with tempfile.TemporaryDirectory() as td:
            # 1. Ambiguity refusal (target matches twice)
            ambig_path = os.path.join(td, "ambig.txt")
            ambig_content = "item=duplicate\nother=data\nitem=duplicate\n"

            samples_ambig = []
            for _ in range(repetitions):
                with open(ambig_path, "w", encoding="utf-8") as f:
                    f.write(ambig_content)
                dur, code, out, err = self.run_direct_cli(
                    ["edit", "ambig.txt", "--search", "item=duplicate", "--replace", "item=unique", "--json"],
                    cwd=td
                )
                with open(ambig_path, "r", encoding="utf-8") as f:
                    content = f.read()
                # Content MUST NOT have changed
                if content != ambig_content:
                    self.wrong_applied += 1
                cert = json.loads(out) if out else {}
                assert cert.get("outcome") == "REFUSED", f"expected REFUSED, got {cert}"
                assert cert.get("reason_code") == "TARGET_AMBIGUOUS"
                samples_ambig.append(dur)
            suite["ambiguity_refusal"] = compute_stats(samples_ambig)

            # 2. Stale pre-hash refusal via mutate --request
            stale_path = os.path.join(td, "stale.txt")
            stale_content = "line 1\nline 2\n"
            req_path = os.path.join(td, "stale_request.json")

            req_json = {
                "version": "1.4.0",
                "request_id": "test-stale",
                "allow_generated": False,
                "allow_newline_conversion": False,
                "file_path": "stale.txt",
                "namespace": {"type": "native"},
                "expected_pre_hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "region_guard": None,
                "candidate_guard": None,
                "cardinality": {"type": "exactly_one"},
                "budget": {
                    "max_files": 1,
                    "max_matches": 1,
                    "max_changed_regions": 1,
                    "max_changed_lines": None,
                    "max_changed_bytes": 100,
                    "allowed_path_prefixes": []
                },
                "operation": {
                    "provider": "text",
                    "operation": {
                        "type": "replace",
                        "target": "line 1",
                        "replacement": "line 1 modified"
                    }
                }
            }
            with open(req_path, "w", encoding="utf-8") as f:
                json.dump(req_json, f)

            samples_stale = []
            for _ in range(repetitions):
                with open(stale_path, "w", encoding="utf-8") as f:
                    f.write(stale_content)
                dur, code, out, err = self.run_direct_cli(
                    ["mutate", "--request", "stale_request.json"],
                    cwd=td
                )
                with open(stale_path, "r", encoding="utf-8") as f:
                    content = f.read()
                if content != stale_content:
                    self.wrong_applied += 1
                cert = json.loads(out) if out else {}
                assert cert.get("outcome") == "REFUSED"
                assert cert.get("reason_code") == "STALE_IDENTITY"
                samples_stale.append(dur)
            suite["stale_hash_refusal"] = compute_stats(samples_stale)

        self.results["refusals_and_safety"] = suite

    # Benchmark: Persistent MCP Connection (stdio JSON-RPC)
    def bench_mcp_suite(self, repetitions=15):
        suite = {}
        with tempfile.TemporaryDirectory() as td:
            # Spawn persistent MCP server process in td
            p = subprocess.Popen([self.exe, "mcp"], cwd=td, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            msg_id = 1

            def mcp_call(method, params):
                nonlocal msg_id
                req = json.dumps({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": params}) + "\n"
                msg_id += 1
                t0 = time.perf_counter()
                p.stdin.write(req)
                p.stdin.flush()
                line = p.stdout.readline()
                t1 = time.perf_counter()
                dur = (t1 - t0) * 1000.0
                resp = json.loads(line)
                return dur, resp

            # 1. Connection Establishment: initialize + tools/list
            t_init_0 = time.perf_counter()
            dur_init, init_resp = mcp_call("initialize", {})
            dur_list, list_resp = mcp_call("tools/list", {})
            t_init_1 = time.perf_counter()
            suite["mcp_connection_establishment_total"] = round((t_init_1 - t_init_0) * 1000.0, 3)
            suite["mcp_step1_initialize"] = round(dur_init, 3)
            suite["mcp_step2_tools_list"] = round(dur_list, 3)

            # 2. Subsequent Read-only tool call (threadmoth_orient)
            samples_orient = []
            for _ in range(repetitions):
                dur, resp = mcp_call("tools/call", {"name": "threadmoth_orient", "arguments": {}})
                assert "result" in resp, f"orient failed: {resp}"
                samples_orient.append(dur)
            suite["mcp_orient_tool_call_warm"] = compute_stats(samples_orient)

            # 3. Subsequent Read-only tool call (threadmoth_capabilities)
            samples_caps = []
            for _ in range(repetitions):
                dur, resp = mcp_call("tools/call", {"name": "threadmoth_capabilities", "arguments": {}})
                assert "result" in resp
                samples_caps.append(dur)
            suite["mcp_capabilities_tool_call_warm"] = compute_stats(samples_caps)

            # 4. Small File Exact Edit via MCP:
            # Route 1: One-call threadmoth_edit / threadmoth_exact_replace
            fixture_path = os.path.join(td, "mcp_small.txt")
            template = "header=info\nunique_token_for_mcp=original\ntrailer=info\n"
            samples_mcp_exact = []
            for _ in range(repetitions):
                with open(fixture_path, "w", encoding="utf-8") as f:
                    f.write(template)
                dur, resp = mcp_call("tools/call", {
                    "name": "threadmoth_exact_replace",
                    "arguments": {
                        "file_path": "mcp_small.txt",
                        "old": "unique_token_for_mcp=original",
                        "new": "unique_token_for_mcp=replaced"
                    }
                })
                with open(fixture_path, "r", encoding="utf-8") as f:
                    content = f.read()
                if "unique_token_for_mcp=replaced" not in content:
                    self.wrong_applied += 1
                cert = resp["result"]["structuredContent"]
                assert cert.get("outcome") == "APPLIED"
                samples_mcp_exact.append(dur)
            suite["mcp_exact_replace_onecall_warm"] = compute_stats(samples_mcp_exact)

            # Route 2: MCP Prepare/Apply (threadmoth_prepare_text -> threadmoth_apply_plan)
            samples_mcp_prep_apply_total = []
            samples_mcp_prep = []
            samples_mcp_apply = []
            for _ in range(repetitions):
                with open(fixture_path, "w", encoding="utf-8") as f:
                    f.write(template)
                # prepare
                dur_prep, resp_prep = mcp_call("tools/call", {
                    "name": "threadmoth_prepare_text",
                    "arguments": {
                        "file_path": "mcp_small.txt",
                        "edits": [{"old": "unique_token_for_mcp=original", "new": "unique_token_for_mcp=replaced"}]
                    }
                })
                assert "result" in resp_prep
                plan = resp_prep["result"]["structuredContent"]["plan"]
                # apply
                dur_app, resp_app = mcp_call("tools/call", {
                    "name": "threadmoth_apply_plan",
                    "arguments": plan
                })
                with open(fixture_path, "r", encoding="utf-8") as f:
                    content = f.read()
                if "unique_token_for_mcp=replaced" not in content:
                    self.wrong_applied += 1
                cert = resp_app["result"]["structuredContent"]
                assert cert.get("outcome") == "APPLIED"
                samples_mcp_prep.append(dur_prep)
                samples_mcp_apply.append(dur_app)
                samples_mcp_prep_apply_total.append(dur_prep + dur_app)

            suite["mcp_prepare_and_apply_route_total"] = compute_stats(samples_mcp_prep_apply_total)
            suite["mcp_step1_prepare_text"] = compute_stats(samples_mcp_prep)
            suite["mcp_step2_apply_plan"] = compute_stats(samples_mcp_apply)

            # 5. MCP Structured edit: threadmoth_set_value
            json_path = os.path.join(td, "mcp_data.json")
            with open(json_path, "w", encoding="utf-8") as f:
                json.dump({"status": "initial", "count": 1}, f)
            samples_mcp_set_value = []
            for _ in range(repetitions):
                with open(json_path, "w", encoding="utf-8") as f:
                    json.dump({"status": "initial", "count": 1}, f)
                dur, resp = mcp_call("tools/call", {
                    "name": "threadmoth_set_value",
                    "arguments": {
                        "file_path": "mcp_data.json",
                        "selector": "status",
                        "value": "updated_via_mcp"
                    }
                })
                assert "result" in resp
                cert = resp["result"]["structuredContent"]
                assert cert.get("outcome") == "APPLIED"
                samples_mcp_set_value.append(dur)
            suite["mcp_set_value_warm"] = compute_stats(samples_mcp_set_value)

            # 6. MCP Refusals: Ambiguity refusal
            ambig_mcp_path = os.path.join(td, "ambig_mcp.txt")
            ambig_content = "target_dup=x\ntarget_dup=x\n"
            samples_mcp_ambig = []
            for _ in range(repetitions):
                with open(ambig_mcp_path, "w", encoding="utf-8") as f:
                    f.write(ambig_content)
                dur, resp = mcp_call("tools/call", {
                    "name": "threadmoth_exact_replace",
                    "arguments": {
                        "file_path": "ambig_mcp.txt",
                        "old": "target_dup=x",
                        "new": "target_dup=y"
                    }
                })
                with open(ambig_mcp_path, "r", encoding="utf-8") as f:
                    c = f.read()
                if c != ambig_content:
                    self.wrong_applied += 1
                cert = resp["result"]["structuredContent"]
                assert cert.get("outcome") == "REFUSED"
                samples_mcp_ambig.append(dur)
            suite["mcp_ambiguity_refusal"] = compute_stats(samples_mcp_ambig)

            # 7. MCP Inspect & Outline Cache Verification
            inspect_rs_path = os.path.join(td, "inspect_target.rs")
            with open(inspect_rs_path, "w", encoding="utf-8") as f:
                f.write("fn calculate() -> i32 { 1 + 1 }\npub fn process_data(input: &str) -> String { input.to_uppercase() }\n")
            samples_inspect_derived = []
            samples_inspect_hit = []
            for _ in range(repetitions):
                # Touch file to force new hash/derivation
                with open(inspect_rs_path, "w", encoding="utf-8") as f:
                    f.write(f"fn calculate() -> i32 {{ {time.time()} as i32 }}\npub fn process_data(input: &str) -> String {{ input.to_uppercase() }}\n")
                dur_der, resp_der = mcp_call("tools/call", {
                    "name": "threadmoth_inspect",
                    "arguments": {
                        "file_path": "inspect_target.rs",
                        "view": "outline"
                    }
                })
                outline_der = resp_der["result"]["structuredContent"]["outline"]
                assert outline_der.get("reuse") == "derived"
                samples_inspect_derived.append(dur_der)

                # Second call on identical file should hit cache
                dur_hit, resp_hit = mcp_call("tools/call", {
                    "name": "threadmoth_inspect",
                    "arguments": {
                        "file_path": "inspect_target.rs",
                        "view": "outline"
                    }
                })
                outline_hit = resp_hit["result"]["structuredContent"]["outline"]
                assert outline_hit.get("reuse") == "cache_hit"
                samples_inspect_hit.append(dur_hit)

            suite["mcp_inspect_outline_derived"] = compute_stats(samples_inspect_derived)
            suite["mcp_inspect_outline_cache_hit"] = compute_stats(samples_inspect_hit)

            # 8. MCP Repeated 20 files in sequence
            samples_mcp_20_files = []
            for _ in range(5):
                for i in range(20):
                    with open(os.path.join(td, f"mcp_file_{i}.txt"), "w", encoding="utf-8") as f:
                        f.write(f"prefix\ntoken_{i}\nsuffix\n")
                t0 = time.perf_counter()
                for i in range(20):
                    dur, resp = mcp_call("tools/call", {
                        "name": "threadmoth_exact_replace",
                        "arguments": {
                            "file_path": f"mcp_file_{i}.txt",
                            "old": f"token_{i}",
                            "new": f"done_{i}"
                        }
                    })
                    assert resp["result"]["structuredContent"]["outcome"] == "APPLIED"
                t1 = time.perf_counter()
                for i in range(20):
                    with open(os.path.join(td, f"mcp_file_{i}.txt"), "r", encoding="utf-8") as f:
                        txt = f.read()
                    if f"done_{i}" not in txt:
                        self.wrong_applied += 1
                samples_mcp_20_files.append((t1 - t0) * 1000.0)
            suite["mcp_repeated_20_files_sequence"] = compute_stats(samples_mcp_20_files)

            # Clean shutdown of MCP server
            p.stdin.close()
            p.terminate()

        self.results["persistent_mcp"] = suite

    def bench_in_process_suite(self):
        """Runs built-in in-process benchmarks (dry-run engine) and torture safety suite."""
        suite = {}
        # 1. Standard / Tough in-process benchmark
        try:
            cmd = [self.exe, "benchmark", "--tough", "--json"]
            out = subprocess.check_output(cmd, text=True)
            data = json.loads(out)
            self.wrong_applied += data.get("wrong_applied", 0)
            suite["tough_report"] = data
        except Exception as e:
            suite["tough_error"] = str(e)

        # 2. Torture safety suite
        try:
            cmd = [self.exe, "benchmark", "--torture", "--json"]
            out = subprocess.check_output(cmd, text=True)
            data = json.loads(out)
            if data.get("state") != "PASS" or data.get("failed", 0) > 0:
                self.wrong_applied += 1
            suite["torture_report"] = data
        except Exception as e:
            suite["torture_error"] = str(e)

        self.results["in_process_benchmarks"] = suite

    def run_all(self, repetitions=15):
        print(f"Starting benchmark run for: {self.exe}", flush=True)
        print("  Phase 1: Startup & Shell invocations...", flush=True)
        self.bench_startup_suite(repetitions)
        print("  Phase 2: Small file exact edit workflows...", flush=True)
        self.bench_small_edit_suite(repetitions)
        print("  Phase 3: Structured edits...", flush=True)
        self.bench_structured_suite(repetitions)
        print("  Phase 4: Larger file edit (1MB)...", flush=True)
        self.bench_large_file_suite(10)
        print("  Phase 5: Repeated small files (20 files)...", flush=True)
        self.bench_repeated_small_files(20, 5)
        print("  Phase 6: Refusals and safety checks...", flush=True)
        self.bench_refusals_suite(repetitions)
        print("  Phase 7: Persistent MCP connection & calls...", flush=True)
        self.bench_mcp_suite(repetitions)
        print("  Phase 8: Built-in in-process benchmarks & torture suite...", flush=True)
        self.bench_in_process_suite()
        print(f"Completed benchmark run. Wrong applied: {self.wrong_applied}", flush=True)

def main():
    import argparse
    parser = argparse.ArgumentParser(description="Threadmoth Agent Latency Benchmark")
    parser.add_argument("--exe", default=r"target\release\threadmoth.exe", help="Path to threadmoth.exe")
    parser.add_argument("--out", default=r"docs\agent_latency_baseline.json", help="Output JSON path")
    parser.add_argument("--repetitions", type=int, default=15, help="Repetitions for warm runs")
    args = parser.parse_args()

    exe_path = os.path.abspath(args.exe)
    if not os.path.exists(exe_path):
        print(f"Error: Executable not found at {exe_path}")
        sys.exit(1)

    runner = BenchmarkRunner(exe_path)
    runner.run_all(repetitions=args.repetitions)

    metadata = get_system_metadata(exe_path)
    full_report = {
        "metadata": metadata,
        "wrong_applied": runner.wrong_applied,
        "results": runner.results,
    }

    out_path = os.path.abspath(args.out)
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(full_report, f, indent=2)
    print(f"Saved benchmark report to: {out_path}")

if __name__ == "__main__":
    main()
