"""Run test signals through LV2 plugins with lv2apply.

Cases live in a JSON file: {"name": {"uri": "...", "controls": {"sym": value, ...}}, ...}
"ref" as the uri runs refcomp.py instead, with controls T R W attack release (makeup).

Usage: uv run python run_lv2.py CASES.json SIG_DIR OUT_DIR [signal ...]
Writes OUT_DIR/<case>/<signal>.wav
"""
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).parent


def run_case(name, case, sig_dir, out_dir, signals):
    dst = out_dir / name
    dst.mkdir(parents=True, exist_ok=True)
    for s in signals:
        src = sig_dir / f"{s}.wav"
        out = dst / f"{s}.wav"
        if case["uri"] == "ref":
            c = case["controls"]
            args = [str(c[k]) for k in ("T", "R", "W", "attack", "release")] + [str(c.get("makeup", 0))]
            cmd = [sys.executable, str(HERE / "refcomp.py"), str(src), str(out), *args]
        else:
            cmd = ["lv2apply", "-i", str(src), "-o", str(out)]
            for sym, val in case.get("controls", {}).items():
                cmd += ["-c", sym, str(val)]
            cmd.append(case["uri"])
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0 and "unsupported type" in r.stderr:
            # lv2apply cannot run plugins with atom ports; use our lilv host instead.
            cmd = [sys.executable, str(HERE / "lv2host.py"), case["uri"], str(src), str(out),
                   *[f"{k}={v}" for k, v in case.get("controls", {}).items()]]
            r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            print(f"{name}/{s}: failed: {r.stderr.strip()[:300]}")


def main():
    cases = json.loads(Path(sys.argv[1]).read_text())
    sig_dir, out_dir = Path(sys.argv[2]), Path(sys.argv[3])
    signals = sys.argv[4:] or ["ramp", "bursts", "lowsine"]
    for name, case in cases.items():
        run_case(name, case, sig_dir, out_dir, signals)
        print(f"done {name}")


if __name__ == "__main__":
    main()
