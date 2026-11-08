#!/usr/bin/env python3
"""Write THIRD_PARTY_LICENSES.txt for a download: every crate from crates.io or git built into the program, with its
license expression and the license texts its package ships (LICENSE*, COPYING*, NOTICE*).

Run on the build machine from the repo root (cargo and the registry sources are there):
  python3 tools/third_party.py app/Cargo.toml OUT.txt [TARGET]
TARGET (for example x86_64-pc-windows-gnu) limits the list to what that platform builds.
"""
import json
import os
import subprocess
import sys

manifest, out = sys.argv[1], sys.argv[2]
target = sys.argv[3] if len(sys.argv) > 3 else None
cmd = ["cargo", "metadata", "--format-version", "1", "--manifest-path", manifest]
if target:
    cmd += ["--filter-platform", target]
meta = json.loads(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout)

# only the packages the build reaches (normal dependencies of the workspace's own packages)
pk = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
seen, todo = set(), list(meta["workspace_members"])
while todo:
    i = todo.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        if any(k["kind"] in (None, "build") for k in d["dep_kinds"]):
            todo.append(d["pkg"])

rows = sorted((pk[i] for i in seen if pk[i]["source"]), key=lambda p: (p["name"], p["version"]))
with open(out, "w", encoding="utf-8") as f:
    f.write("Third-party software built into stream-mic\n\n")
    for p in rows:
        f.write(f"{p['name']} {p['version']}  {p['license'] or 'see the text below'}  {p.get('repository') or ''}\n")
    f.write("\n")
    done = set()
    for p in rows:
        d = os.path.dirname(p["manifest_path"])
        files = sorted(x for x in os.listdir(d) if x.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")))
        if p["license_file"]:
            files.append(os.path.relpath(os.path.join(d, p["license_file"]), d))
        for x in dict.fromkeys(files):
            path = os.path.join(d, x)
            if not os.path.isfile(path):
                continue
            text = open(path, encoding="utf-8", errors="replace").read().strip()
            key = (x.upper().split(".")[0], text)
            f.write("=" * 78 + f"\n{p['name']} {p['version']}: {x}\n" + "=" * 78 + "\n")
            # the same standard text (an unchanged Apache-2.0, say) is printed once and referred to after
            f.write(("(same text as above)" if key in done else text) + "\n\n")
            done.add(key)
print(f"{len(rows)} crates -> {out}")
