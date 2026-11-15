import json
import sys

def load(p):
    with open(p, 'r') as f:
        return json.load(f)

built = load('docs/agent_latency_baseline_built.json')
inst = load('docs/agent_latency_baseline_installed.json')

print('=== ENVIRONMENT & SYSTEM METADATA ===')
print(f'OS: {built["metadata"].get("os_name")} (Build {built["metadata"].get("os_build")})')
print(f'CPU: {built["metadata"].get("cpu_name")} ({built["metadata"].get("cpu_cores")} cores, {built["metadata"].get("cpu_logical")} logical)')
print(f'Storage: C: {built["metadata"].get("c_fs")} ({built["metadata"].get("c_free_gb")} GB free / {built["metadata"].get("c_size_gb")} GB)')
print(f'Defender: AntivirusEnabled={built["metadata"].get("antivirus_enabled")}, RealTimeProtection={built["metadata"].get("rtp_enabled")}')
print(f'Built Binary: size={built["metadata"].get("exe_size")} bytes, sha256={built["metadata"].get("exe_sha256")[:16]}..., git_sha={built["metadata"].get("git_sha")[:10]}')
print(f'Installed Binary: size={inst["metadata"].get("exe_size")} bytes, sha256={inst["metadata"].get("exe_sha256")[:16]}...')
print(f'Correctness standard: wrong_applied = {built.get("wrong_applied")} (built), {inst.get("wrong_applied")} (installed)')
print()

def print_section(title, key):
    print(f'=== {title} ===')
    b_sec = built['results'].get(key, {})
    i_sec = inst['results'].get(key, {})
    for k in b_sec:
        b_val = b_sec[k]
        i_val = i_sec.get(k, {})
        if isinstance(b_val, dict) and 'median_ms' in b_val:
            print(f'  {k:45}: Built med={b_val["median_ms"]:7.2f} ms (p95={b_val["p95_ms"]:7.2f}) | Inst med={i_val.get("median_ms", 0):7.2f} ms (p95={i_val.get("p95_ms", 0):7.2f})')
        else:
            print(f'  {k:45}: Built={b_val} | Inst={i_val}')
    print()

print_section('Startup & Shell Invocations', 'startup_and_shell')
print_section('Small File Edit Workflows', 'small_file_workflows')
print_section('Structured Edits', 'structured_edits')
print_section('Large File (1MB)', 'large_file')
print_section('Repeated Small Files (20 files)', 'repeated_small_files')
print_section('Refusals and Safety', 'refusals_and_safety')
print_section('Persistent MCP Connection', 'persistent_mcp')
