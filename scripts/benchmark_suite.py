import time
import subprocess
import os
import tempfile
import json
import statistics

EXE = os.path.abspath(r"target\release\threadmoth.exe")

def run_cmd(args, cwd=None, input_data=None):
    t0 = time.perf_counter()
    p = subprocess.run([EXE] + args, cwd=cwd, input=input_data, capture_output=True)
    t1 = time.perf_counter()
    return (t1 - t0) * 1000.0, p.returncode, p.stdout, p.stderr

def bench_repeated(name, fn, runs=10):
    times = []
    for _ in range(runs):
        ms = fn()
        times.append(ms)
    avg = statistics.mean(times)
    median = statistics.median(times)
    p90 = sorted(times)[int(len(times) * 0.9)]
    min_t = min(times)
    print(f"{name:40}: avg {avg:6.2f} ms | min {min_t:6.2f} ms | med {median:6.2f} ms | p90 {p90:6.2f} ms")
    return {"avg": avg, "min": min_t, "median": median, "p90": p90}

results = {}

# 1. Startup & Invocation
# 1a. Initial launch (cold invocation)
dur, code, out, err = run_cmd(["--version"])
assert code == 0
print(f"{'1a. Initial launch (--version)':40}: {dur:6.2f} ms")
results["initial_launch_cold"] = {"dur": dur}

# 1b. Steady-state repeated execution (warm process cache)
def bench_startup():
    dur, code, out, err = run_cmd(["--version"])
    assert code == 0
    return dur
results["steady_state_version_warm"] = bench_repeated("1b. Steady-state execution (--version)", bench_startup, 20)

with tempfile.TemporaryDirectory() as td:
    # 2. Simple one-line edit on small file (1 KB)
    small_path = os.path.join(td, "small.txt")
    with open(small_path, "wb") as f:
        f.write(b"key=original\nother=value\n" * 49 + b"unique_key=original\n")
    
    def bench_small_edit():
        dur, code, out, err = run_cmd(["edit", "small.txt", "--search", "unique_key=original", "--replace", "unique_key=updated"], cwd=td)
        if code != 0:
            print("FAILED small_edit:", code, out.decode(), err.decode())
            assert code == 0
        # restore
        with open(small_path, "wb") as f:
            f.write(b"key=original\nother=value\n" * 49 + b"unique_key=original\n")
        return dur
    results["small_file_edit"] = bench_repeated("2. Small file (1KB) edit", bench_small_edit, 15)

    # 3. Medium file (100 KB CRLF)
    med_path = os.path.join(td, "med.txt")
    with open(med_path, "wb") as f:
        f.write(b"line number here for testing CRLF\r\n" * 3000)
        f.write(b"TARGET_UNIQUE_LINE\r\n")
        f.write(b"trailing line after target\r\n" * 500)

    def bench_med_crlf_edit():
        dur, code, out, err = run_cmd(["edit", "med.txt", "--search", "TARGET_UNIQUE_LINE", "--replace", "REPLACED_UNIQUE_LINE"], cwd=td)
        if code != 0:
            print("FAILED med_crlf_edit:", code, out.decode(), err.decode())
            assert code == 0
        with open(med_path, "wb") as f:
            f.write(b"line number here for testing CRLF\r\n" * 3000)
            f.write(b"TARGET_UNIQUE_LINE\r\n")
            f.write(b"trailing line after target\r\n" * 500)
        return dur
    results["med_file_crlf_edit"] = bench_repeated("3. Medium file (100KB CRLF) edit", bench_med_crlf_edit, 10)

    # 4. Preview vs Mutation
    def bench_preview():
        dur, code, out, err = run_cmd(["edit", "med.txt", "--search", "TARGET_UNIQUE_LINE", "--replace", "REPLACED_UNIQUE_LINE", "--plan"], cwd=td)
        if code != 0:
            print("FAILED preview:", code, out.decode(), err.decode())
            assert code == 0
        return dur
    results["preview_plan"] = bench_repeated("4. Preview (--plan) mode", bench_preview, 10)

    # 5. Multi-file edit document (3 files)
    f1 = os.path.join(td, "f1.txt")
    f2 = os.path.join(td, "f2.txt")
    f3 = os.path.join(td, "f3.txt")
    doc_path = os.path.join(td, "changes.tm")
    
    doc_content = f"""*** File: f1.txt
<<<<<<< SEARCH
alpha=1
=======
alpha=2
>>>>>>> REPLACE

*** File: f2.txt
<<<<<<< SEARCH
beta=1
=======
beta=2
>>>>>>> REPLACE

*** File: f3.txt
<<<<<<< SEARCH
gamma=1
=======
gamma=2
>>>>>>> REPLACE
"""
    with open(doc_path, "w", encoding="utf-8") as df:
        df.write(doc_content)

    def bench_multifile():
        with open(f1, "w") as f: f.write("alpha=1\n")
        with open(f2, "w") as f: f.write("beta=1\n")
        with open(f3, "w") as f: f.write("gamma=1\n")
        dur, code, out, err = run_cmd(["edit", "changes.tm"], cwd=td)
        if code != 0:
            print("FAILED multifile:", code, out.decode(), err.decode())
            assert code == 0
        return dur
    results["multifile_edit_doc"] = bench_repeated("5. Multi-file (3 files) edit document", bench_multifile, 10)

out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "benchmark_results.json")
with open(out_path, "w") as out:
    json.dump(results, out, indent=2)
print(f"Saved benchmark results to {out_path}")
