#!/usr/bin/env python3
"""Black-box Threadmoth evaluator. Python standard library only."""
import argparse, base64, hashlib, json, os, shutil, subprocess, tempfile, time
from pathlib import Path

V = "1.4.0"
def sha(b): return hashlib.sha256(b).hexdigest()
def req(path, provider, operation, rid, pre_hash=None):
    return {"version":V,"request_id":rid,"file_path":path,"cardinality":{"type":"exactly_one"},"budget":{"max_files":1,"max_matches":1,"allowed_path_prefixes":[]},"operation":{"provider":provider,"operation":operation},"expected_pre_hash":pre_hash}
def cases():
    large = ("row=unchanged\n" * 5000) + "needle\n" + ("tail=preserve\n" * 5000)
    repetitive = "".join(f"token_{i:03}=old\n" for i in range(200))
    repetitive64 = "".join(f"token_{i:03}=old\n" for i in range(64))
    repetitive64_expected = "".join(f"token_{i:03}=new\n" for i in range(64))
    repetitive_expected = "".join(f"token_{i:03}=new\n" for i in range(200))
    sequence64 = req("many.txt", "text", {"type":"replace_sequence", "edits":[{"old":f"token_{i:03}=old", "new":f"token_{i:03}=new"} for i in range(64)]}, "sixty-four")
    sequence64["budget"]["max_matches"] = 64
    sequence200 = req("many.txt", "text", {"type":"replace_sequence", "edits":[{"old":f"token_{i:03}=old", "new":f"token_{i:03}=new"} for i in range(200)]}, "two-hundred")
    sequence200["budget"]["max_matches"] = 200

    many_files = {f"pkg/mod{i:02}.txt":f"old_{i:02}\n" for i in range(20)}
    many_expected = {name:value.replace("old_", "new_", 1) for name,value in many_files.items()}
    multi = {"version":V,"transaction_id":"lab-twenty-files","budget":{"max_files":20,"max_matches":20,"allowed_path_prefixes":[]},"requests":[req(name,"text",{"type":"replace","target":f"old_{i:02}","replacement":f"new_{i:02}"},f"tx-{i:02}") for i,name in enumerate(many_files)]}
    return [
      {"id":"utf8-multibyte-offset","files":{"a.txt":"# 🧪 section\nname=old\n"},"request":req("a.txt","text",{"type":"replace","target":"old","replacement":"new"},"unicode"),"expect":"APPLIED","expected":{"a.txt":"# 🧪 section\nname=new\n"}},
      {"id":"sixty-four-replacements","files":{"many.txt":repetitive64},"request":sequence64,"expect":"APPLIED","expected":{"many.txt":repetitive64_expected}},
      {"id":"two-hundred-replacements-limit","files":{"many.txt":repetitive},"request":sequence200,"expect":"REFUSED","expected":{"many.txt":repetitive}},
      {"id":"twenty-file-transaction","files":many_files,"transaction":multi,"expect":"APPLIED","expected":many_expected},
      {"id":"precise-one","files":{"a.txt":"before\nold\nafter\n"},"request":req("a.txt","text",{"type":"replace","target":"old","replacement":"new"},"precise"),"expect":"APPLIED","expected":{"a.txt":"before\nnew\nafter\n"}},
      {"id":"duplicate-refusal","files":{"a.txt":"same\nkeep\nsame\n"},"request":req("a.txt","text",{"type":"replace","target":"same","replacement":"changed"},"ambiguous"),"expect":"REFUSED","expected":{"a.txt":"same\nkeep\nsame\n"}},
      {"id":"crlf-preservation","files":{"a.txt":"first\r\nold\r\nlast\r\n"},"request":req("a.txt","text",{"type":"replace","target":"old","replacement":"new"},"crlf"),"expect":"APPLIED","expected":{"a.txt":"first\r\nnew\r\nlast\r\n"}},
      {"id":"large-distant-edit","files":{"large.txt":large},"request":req("large.txt","text",{"type":"replace","target":"needle","replacement":"updated"},"large"),"expect":"APPLIED","expected":{"large.txt":large.replace("needle","updated",1)}},
      {"id":"json-structural-set","files":{"config.json":"{\n  \"name\": \"old\",\n  \"retain\": true\n}\n"},"request":req("config.json","json",{"type":"set","path":"$.name","value":"new"},"json"),"expect":"APPLIED","expected":{"config.json":"{\n  \"name\": \"new\",\n  \"retain\": true\n}\n"}},
      {"id":"syntax-node-replace","files":{"lib.rs":"fn target() {\n    // keep this comment\n    let x = 1;\n}\n\nfn similar() { let x = 1; }\n"},"request":req("lib.rs","code",{"type":"replace_node","language":"rust","target":"fn target() {\n    // keep this comment\n    let x = 1;\n}","replacement":"fn target() {\n    // keep this comment\n    let x = 2;\n}"},"syntax"),"expect":"APPLIED","expected":{"lib.rs":"fn target() {\n    // keep this comment\n    let x = 2;\n}\n\nfn similar() { let x = 1; }\n"}},
      {"id":"multi-file-transaction","files":{"a.txt":"old-a\n","b.txt":"old-b\n"},"transaction":{"version":V,"transaction_id":"lab-multi-file","budget":{"max_files":2,"max_matches":2,"allowed_path_prefixes":[]},"requests":[req("a.txt","text",{"type":"replace","target":"old-a","replacement":"new-a"},"tx-a"),req("b.txt","text",{"type":"replace","target":"old-b","replacement":"new-b"},"tx-b")]},"expect":"APPLIED","expected":{"a.txt":"new-a\n","b.txt":"new-b\n"}},
      {"id":"stale-prehash-refusal","files":{"a.txt":"current\n"},"request":req("a.txt","text",{"type":"replace","target":"current","replacement":"changed"},"stale","0"*64),"expect":"REFUSED","expected":{"a.txt":"current\n"}},
      {"id":"binary-refusal","files":{"binary.bin":{"base64":"AAECAwQ="}},"request":req("binary.bin","text",{"type":"replace","target":"x","replacement":"y"},"binary"),"expect":"REFUSED","expected":{"binary.bin":{"base64":"AAECAwQ="}}},
      {"id":"preview-does-not-write","mode":"preview","files":{"a.txt":"before old after\n"},"request":req("a.txt","text",{"type":"replace","target":"old","replacement":"new"},"preview"),"expect":"APPLIED","expected":{"a.txt":"before old after\n"}}
    ]
def material(v):
    if isinstance(v,str): return v.encode()
    return base64.b64decode(v["base64"])
def run(cmd, cwd, timeout):
    t=time.perf_counter(); p=subprocess.run(cmd,cwd=cwd,capture_output=True,text=True,timeout=timeout); ms=(time.perf_counter()-t)*1000
    try: obj=json.loads(p.stdout)
    except Exception: obj=None
    return p,ms,obj

def main():
    ap=argparse.ArgumentParser(description=__doc__); ap.add_argument("--executable",default=os.environ.get("THREADMOTH","threadmoth")); ap.add_argument("--report",default="threadmoth-lab-report.json"); ap.add_argument("--timeout",type=int,default=60); ap.add_argument("--keep-workspaces",action="store_true"); a=ap.parse_args()
    exe=shutil.which(a.executable) or a.executable
    version=run([exe,"--version"],Path.cwd(),a.timeout)[0].stdout.strip()
    doctor=run([exe,"doctor","--json"],Path.cwd(),a.timeout)[2]
    caps=run([exe,"capabilities","--json","--all"],Path.cwd(),a.timeout)[2]
    cases_out=[]; temp=tempfile.mkdtemp(prefix="threadmoth-lab-")
    try:
      for c in cases():
        root=Path(temp)/c["id"]; root.mkdir()
        initial={}
        for name,val in c["files"].items():
          data=material(val); p=root/name; p.parent.mkdir(parents=True,exist_ok=True); p.write_bytes(data); initial[name]=data
        request=c.get("transaction") or c["request"]; request_path=root/("transaction.json" if "transaction" in c else "request.json"); request_path.write_text(json.dumps(request,ensure_ascii=False),encoding="utf-8")
        cmd=[exe,"transact","--request",request_path.name] if "transaction" in c else [exe,c.get("mode","mutate"),"--request",request_path.name]
        proc,elapsed,obj=run(cmd,root,a.timeout); observed=(obj or {}).get("outcome","UNKNOWN")
        final={}
        for name in c["expected"]:
          p=root/name; final[name]=p.read_bytes() if p.exists() else None
        expected={k:material(v) for k,v in c["expected"].items()}
        byte_ok=all(final.get(k)==v for k,v in expected.items()) and set(final)==set(expected)
        pass_case=proc.returncode in (0,2) and observed==c["expect"] and byte_ok
        cases_out.append({"id":c["id"],"state":"PASS" if pass_case else "FAIL","expected_outcome":c["expect"],"observed_outcome":observed,"exit_code":proc.returncode,"elapsed_ms":round(elapsed,3),"invocations":1,"independent_bytes_match":byte_ok,"original_sha256":{k:sha(v) for k,v in initial.items()},"expected_sha256":{k:sha(v) for k,v in expected.items()},"final_sha256":{k:(sha(v) if v is not None else None) for k,v in final.items()},"certificate":obj,"stderr":proc.stderr[-2000:]})
      report={"format":"threadmoth-toolbelt-lab/v1","state":"PASS" if all(x["state"]=="PASS" for x in cases_out) else "FAIL","executable":exe,"version":version,"doctor":doctor,"capabilities":caps,"host":{"os":os.name,"python":__import__("platform").python_version()},"summary":{"cases":len(cases_out),"passed":sum(x["state"]=="PASS" for x in cases_out),"failed":sum(x["state"]!="PASS" for x in cases_out),"case_invocations":sum(x["invocations"] for x in cases_out),"metadata_invocations":3,"total_invocations":3+sum(x["invocations"] for x in cases_out),"elapsed_ms":round(sum(x["elapsed_ms"] for x in cases_out),3)},"cases":cases_out}
      Path(a.report).write_text(json.dumps(report,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
      print(json.dumps({"state":report["state"],"report":str(Path(a.report).resolve()),"summary":report["summary"]},indent=2))
      return 0 if report["state"]=="PASS" else 1
    finally:
      if not a.keep_workspaces: shutil.rmtree(temp,ignore_errors=True)
if __name__=="__main__": raise SystemExit(main())