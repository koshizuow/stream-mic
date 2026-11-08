"""Check the Rust de-esser, compressor, expander and limiter against the Python references (build-plan.md step 2):
same output to -100 dB or better, identical at block sizes 1, 128 and 4096, and the limiter's true peak
(measured independently, 4x resample_poly) at or below the ceiling.

Usage: uv run python dyn_check.py CLI WORKDIR SPEECH.wav NOISY.wav
"""
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

from deess_bench import ref_deess
from refcomp import compress
from refexp import expand
from reflim import limit

FS = 48000


def signals(speech, noisy):
    s, fs = sf.read(speech, dtype="float64")
    assert fs == FS
    s = s if s.ndim == 1 else s[:, 0]
    s = s[: 20 * FS] / np.abs(s[: 20 * FS]).max()
    n, fs = sf.read(noisy, dtype="float64")
    n = n if n.ndim == 1 else n[:, 0]
    n = n[: 20 * FS] / np.abs(n[: 20 * FS]).max() * 0.5
    t = np.arange(FS * 6) / FS
    burst = 0.1 * np.sin(2 * np.pi * 1000 * t)
    burst[(t >= 2) & (t < 2.5)] *= 10
    tr = np.arange(40 * FS) / FS
    return {
        "speech": s,
        "speech12": s * 2.0,
        "noisy": n,
        "burst": burst,
        "low": np.sin(2 * np.pi * 100 * np.arange(4 * FS) / FS),
        "isp": 10 ** (-3 / 20) / np.sin(np.pi / 4) * np.sin(2 * np.pi * 12000 * np.arange(4 * FS) / FS + np.pi / 4),
        "ramp": 10 ** ((-60 + 1.5 * tr) / 20) * np.sin(2 * np.pi * 1000 * tr),
    }


# (cli option, cli keys, reference function, signals)
CASES = [
    ("--deess", dict(thr=6, freq=7000, q=0.8, ratio=10, max=12, att=1, rel=15),
     lambda x: ref_deess(x, "tilt", 6, ratio=10, max_gr=12, rel_ms=15, q=0.8), ["speech", "noisy", "ramp"]),
    ("--deess", dict(thr=0, freq=7000, q=0.8, ratio=4, max=6, att=1, rel=40),
     lambda x: ref_deess(x, "tilt", 0, ratio=4, max_gr=6, rel_ms=40, q=0.8), ["speech"]),
    ("--comp", dict(thr=-30, ratio=4, knee=0, att=10, rel=100, makeup=0, schp=0),
     lambda x: compress(x, FS, -30, 4, 0, 10, 100, 0)[0], ["speech", "burst", "low", "ramp"]),
    ("--comp", dict(thr=-24, ratio=3, knee=6, att=10, rel=120, makeup=6, schp=100),
     lambda x: compress(x, FS, -24, 3, 6, 10, 120, 6, 100)[0], ["speech", "low", "ramp"]),
    ("--exp", dict(thr=-54, ratio=2.5, range=12, knee=10, att=3, hold=80, rel=200, det=10, schp=100),
     lambda x: expand(x, FS, -54, 2.5, 12, 10, 3, 80, 200, 10, 100)[0], ["noisy", "burst", "ramp"]),
    ("--exp", dict(thr=-58, ratio=1.7, range=8, knee=10, att=3, hold=150, rel=600, det=10, schp=100),
     lambda x: expand(x, FS, -58, 1.7, 8, 10, 3, 150, 600, 10, 100)[0], ["noisy"]),
    ("--lim", dict(ceil=-6, rel=50, look=2, hold=10, tp=1),
     lambda x: limit(x, FS, -6, 2, 10, 50, "causal")[0], ["speech", "speech12", "burst", "low", "isp"]),
    ("--lim", dict(ceil=-6, rel=50, look=2, hold=10, tp=0),
     lambda x: limit(x, FS, -6, 2, 10, 50, False)[0], ["speech", "isp"]),
]


def run(cli, inp, out, opt, keys, block):
    arg = ",".join(f"{k}={v}" for k, v in keys.items())
    r = subprocess.run([cli, str(inp), str(out), "--block", str(block), opt, arg], check=True, capture_output=True, text=True)
    return sf.read(out, dtype="float64")[0], r.stderr


def main():
    cli, work = sys.argv[1], Path(sys.argv[2])
    work.mkdir(parents=True, exist_ok=True)
    sigs = signals(sys.argv[3], sys.argv[4])
    for k, v in sigs.items():
        sf.write(work / f"dyn_{k}.wav", v.astype(np.float32), FS, subtype="FLOAT")
    ok = True
    for opt, keys, ref, names in CASES:
        print(f"\n{opt} {keys}")
        for name in names:
            x = sf.read(work / f"dyn_{name}.wav", dtype="float64")[0]
            want = ref(x)
            outs = [run(cli, work / f"dyn_{name}.wav", work / f"dyn_out_{b}.wav", opt, keys, b)[0] for b in (1, 128, 4096)]
            same = all(np.array_equal(outs[0], o) for o in outs[1:])
            y = outs[1]
            e = np.sqrt(np.mean((y - want) ** 2)) / max(np.sqrt(np.mean(want ** 2)), 1e-20)
            err = 20 * np.log10(max(e, 1e-20))
            line = f"  {name:9s} error {err:7.1f} dB  blocks identical {same}"
            good = err < -100 and same
            if opt == "--lim":
                thr = 10 ** (keys["ceil"] / 20)
                tp = 20 * np.log10(np.abs(resample_poly(y, 4, 1)[FS:-FS]).max() / thr)
                sp = 20 * np.log10(np.abs(y).max() / thr)
                line += f"  sample peak {sp:+.3f} dB  true peak {tp:+.3f} dB (re ceiling)"
                good &= sp <= 1e-6 and (tp <= 0.1 if keys["tp"] else True)
            ok &= good
            print(line + ("" if good else "  <-- FAIL"))
    print("\nPASS" if ok else "\nFAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
