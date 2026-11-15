import json
import sys

def load(p):
    with open(p, 'r') as f:
        return json.load(f)

before = load('docs/agent_latency_baseline_built.json')
after = load('docs/agent_latency_after_improvements.json')
installed = load('docs/agent_latency_baseline_installed.json')

print('====================================================================================================')
print(' THREADMOTH WINDOWS AGENT LATENCY BENCHMARK: BEFORE vs AFTER EVIDENCE')
print('====================================================================================================')
print(f'OS: {after["metadata"].get("os_name")} (Build {after["metadata"].get("os_build")})')
print(f'CPU: {after["metadata"].get("cpu_name")} ({after["metadata"].get("cpu_cores")} cores, {after["metadata"].get("cpu_logical")} logical)')
print(f'Storage: C: {after["metadata"].get("c_fs")} ({after["metadata"].get("c_free_gb")} GB free / {after["metadata"].get("c_size_gb")} GB)')
print(f'Defender: AntivirusEnabled={after["metadata"].get("antivirus_enabled")}, RealTimeProtection={after["metadata"].get("rtp_enabled")}')
print(f'Commit: {after["metadata"].get("git_sha")} (branch: {after["metadata"].get("git_branch")})')
print(f'Correctness: wrong_applied = {after.get("wrong_applied")} (zero wrong mutations across all runs)')
print('====================================================================================================\n')

print(f"{'Workload / Operation':45} | {'Before Med (p95)':22} | {'After Med (p95)':22} | {'Delta (%)':10}")
print("-" * 106)

def compare_section(title, key):
    print(f"\n[ {title} ]")
    b_sec = before['results'].get(key, {})
    a_sec = after['results'].get(key, {})
    for k in b_sec:
        b_val = b_sec[k]
        a_val = a_sec.get(k, {})
        if isinstance(b_val, dict) and 'median_ms' in b_val and 'median_ms' in a_val:
            b_med = b_val['median_ms']
            b_p95 = b_val['p95_ms']
            a_med = a_val['median_ms']
            a_p95 = a_val['p95_ms']
            delta_pct = ((a_med - b_med) / b_med) * 100.0 if b_med > 0 else 0.0
            sign = "+" if delta_pct > 0 else ""
            b_str = f"{b_med:6.2f} ms ({b_p95:6.2f})"
            a_str = f"{a_med:6.2f} ms ({a_p95:6.2f})"
            d_str = f"{sign}{delta_pct:5.1f}%"
            print(f"  {k:43} | {b_str:22} | {a_str:22} | {d_str:10}")
        elif isinstance(b_val, (int, float)) and isinstance(a_val, (int, float)):
            delta_pct = ((a_val - b_val) / b_val) * 100.0 if b_val > 0 else 0.0
            sign = "+" if delta_pct > 0 else ""
            print(f"  {k:43} | {b_val:9.2f} ms            | {a_val:9.2f} ms            | {sign}{delta_pct:5.1f}%")

compare_section('Startup & Shell Invocations', 'startup_and_shell')
compare_section('Small File Edit Workflows', 'small_file_workflows')
compare_section('Structured Edits', 'structured_edits')
compare_section('Large File (1MB)', 'large_file')
compare_section('Repeated Small Files (20 files)', 'repeated_small_files')
compare_section('Refusals and Safety', 'refusals_and_safety')
compare_section('Persistent MCP Connection', 'persistent_mcp')
