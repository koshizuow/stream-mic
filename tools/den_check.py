"""Check the embedded DeepFilterNet3 stage (build-plan.md step 4): output identical at block sizes 1, 128,
480 and 4096; the latency it reports matches the measured shift; and the output equals the official
deep-filter 0.5.6 program's (same low latency model) once the extra hop of buffering is accounted for.

Usage: uv run python den_check.py CLI DEEP_FILTER_BIN MODEL_LL_TAR NOISY.wav WORKDIR
"""
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

FS = 48000


def lag_of(x, y, max_lag=2400):
    a = x[FS: FS * 6]
    c = [np.dot(a, y[FS + k: FS * 6 + k]) for k in range(0, max_lag)]
    return int(np.argmax(c))


def main():
    cli, dfbin, model, noisy, work = sys.argv[1:6]
    work = Path(work)
    work.mkdir(parents=True, exist_ok=True)
    x, fs = sf.read(noisy, dtype="float64")
    x = (x if x.ndim == 1 else x[:, 0])[: 20 * FS]
    src = work / "den_in.wav"
    sf.write(src, x.astype(np.float32), FS, subtype="FLOAT")
    outs = {}
    for b in (1, 128, 480, 4096):
        r = subprocess.run([cli, str(src), str(work / f"den_{b}.wav"), "--block", str(b), "--denoise", "limit=100"],
                           check=True, capture_output=True, text=True)
        outs[b] = sf.read(work / f"den_{b}.wav", dtype="float64")[0]
        if b == 128:
            print(r.stderr.strip())
    same = all(np.array_equal(outs[1], o) for o in outs.values())
    print("blocks 1/128/480/4096 identical:", same)
    y = outs[128]
    print("measured shift against the input:", lag_of(x, y), "samples")
    tmp = work / "_df"
    tmp.mkdir(exist_ok=True)
    subprocess.run([dfbin, "-m", model, "-o", str(tmp), str(src)], check=True, capture_output=True)
    ref = sf.read(tmp / src.name, dtype="float64")[0]
    k = lag_of(ref, y)
    print("our output against deep-filter's:", k, "samples later")
    n = min(len(ref), len(y) - k) - FS
    a, b = ref[FS:n], y[FS + k:n + k]
    print(f"difference after aligning: {20 * np.log10(np.sqrt(np.mean((a - b) ** 2)) / np.sqrt(np.mean(a ** 2))):.1f} dB")


if __name__ == "__main__":
    main()
