"""De-esser comparison on speech.

  uv run python deess_bench.py CLEAN_DIR WORK_DIR

CLEAN_DIR holds clean 48 kHz voices (the first 4 WAVs: DAPS f1 f3 m1 m3 for speech, the VocalSet set for singing);
20 s of each are joined into one file, peak -3 dBFS.

Every processor is tuned (bisection on its threshold) so the 4.5-10 kHz band of sibilant frames drops by 6 dB on
average at the nominal level. Then, at that setting:
- sib_hf:     the 4.5-10 kHz change on sibilant frames (6 by construction)
- sib_low:    the change below 4 kHz on sibilant frames (a wide-band de-esser ducks the whole voice: lisping)
- other_hf:   the 4.5-10 kHz change on other voiced frames (dulling where there is no s)
- quiet_hf:   sib_hf with the input 15 dB lower (a fixed threshold misses soft sibilance)
- latency

Sibilant frames (10 ms): active (over -50 dBFS), and the 4-12 kHz energy is above the energy below 4 kHz.

Our two reference designs share a dynamic bell (y = x + (g-1)·bp(x), with bp an RBJ band-pass at 7 kHz, Q 0.8, so
g = 1 is exactly the input and nothing is touched between s sounds). They differ in the detector:
- ref_abs: the band-pass level against a fixed threshold (the usual de-esser)
- ref_rel: the band-pass level minus the full-band level (how much of the voice is s right now), so the result does
  not depend on how loud the person speaks. Time domain, no zero-crossing rate: not the Dolby patent's method
  (US 11322170 claims FFT buffers, the ratio to the previous buffer's envelope, and a ZCR threshold together).
"""
import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import lfilter, butter, sosfilt

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
from measure import align  # noqa: E402

FS = 48000
FR = FS // 100  # 10 ms frames


def bp_coef(f0=7000.0, q=0.8):
    w = 2 * np.pi * f0 / FS
    al = np.sin(w) / (2 * q)
    b = np.array([al, 0, -al]) / (1 + al)
    a = np.array([1, -2 * np.cos(w) / (1 + al), (1 - al) / (1 + al)])
    return b, a


def smooth_db(p, att_ms, rel_ms):
    """Power to dB with decoupled attack and release on the power envelope."""
    aa = np.exp(-1 / (att_ms * 1e-3 * FS))
    ar = np.exp(-1 / (rel_ms * 1e-3 * FS))
    out = np.empty_like(p)
    e = 0.0
    for i, v in enumerate(p):
        e = aa * e + (1 - aa) * v if v > e else ar * e + (1 - ar) * v
        out[i] = e
    return 10 * np.log10(out + 1e-20)


def ref_deess(x, mode, thr, ratio=4.0, max_gr=12.0, rel_ms=40.0, q=0.8):
    b, a = bp_coef(q=q)
    s = lfilter(b, a, x)
    ls = smooth_db(s ** 2, 1.0, rel_ms)
    if mode == "rel":
        over = ls - smooth_db(x ** 2, 1.0, rel_ms) - thr
    elif mode == "tilt":
        # s sounds have more energy up high than below 3 kHz; vowels never do
        lp = lfilter(*butter(2, 3000, "lowpass", fs=FS), x)
        over = ls - smooth_db(lp ** 2, 1.0, rel_ms) - thr
    else:
        over = ls - thr
    gr = np.clip(over, 0, None) * (1 - 1 / ratio)
    gr = np.minimum(gr, max_gr)
    g = 10 ** (-gr / 20)
    return x + (g - 1) * s


CASES = {
    "ref_abs": {"ref": "abs", "lo": -70.0, "hi": 0.0},
    "ref_rel": {"ref": "rel", "lo": -30.0, "hi": 10.0},
    "ref_rel_r15": {"ref": "rel", "lo": -30.0, "hi": 10.0, "opt": {"rel_ms": 15.0}},
    "ref_rel_r15_x10": {"ref": "rel", "lo": -30.0, "hi": 10.0, "opt": {"rel_ms": 15.0, "ratio": 10.0}},
    "ref_rel_r15_q2": {"ref": "rel", "lo": -30.0, "hi": 10.0, "opt": {"rel_ms": 15.0, "q": 2.0}},
    "ref_tilt_r15_x10": {"ref": "tilt", "lo": -30.0, "hi": 20.0, "opt": {"rel_ms": 15.0, "ratio": 10.0}},
    "ref_tilt_r15_x4": {"ref": "tilt", "lo": -30.0, "hi": 20.0, "opt": {"rel_ms": 15.0}},
    # our Rust de-esser (STREAM_MIC_CLI): same tilt design as ref_tilt_r15_x10
    "rust_tilt": {"cli": "freq=7000,q=0.8,ratio=10,max=12,att=1,rel=15", "lo": -30.0, "hi": 20.0},
    "calf_wide": {"uri": "http://calf.sourceforge.net/plugins/Deesser", "param": "threshold", "log": True,
                  "lo": 0.000977, "hi": 1.0, "controls": {"mode": 0, "ratio": 4}},
    "calf_split": {"uri": "http://calf.sourceforge.net/plugins/Deesser", "param": "threshold", "log": True,
                   "lo": 0.000977, "hi": 1.0, "controls": {"mode": 1, "ratio": 4}},
    "mda": {"uri": "http://drobilla.net/plugins/mda/DeEss", "param": "thresh", "log": False, "lo": 0.0, "hi": 1.0,
            "controls": {}},
    "lsp_comp_schpf": {"uri": "http://lsp-plug.in/plugins/lv2/compressor_mono", "param": "al", "log": True,
                       "lo": 0.0001, "hi": 1.0,
                       "controls": {"sct": 0, "scm": 1, "shpm": 3, "shpf": 5000, "cr": 4, "at": 1, "rt": 60,
                                    "kn": 1.0, "rrl": 0}},
}


def run(case, val, src, dst):
    if "cli" in case:
        subprocess.run([os.environ["STREAM_MIC_CLI"], str(src), str(dst), "--deess", f"{case['cli']},thr={val}"],
                       check=True, capture_output=True)
        return
    x, _ = sf.read(src, dtype="float64")
    if "ref" in case:
        sf.write(dst, ref_deess(x, case["ref"], val, **case.get("opt", {})).astype(np.float32), FS, subtype="FLOAT")
        return
    ctl = dict(case["controls"], **{case["param"]: val})
    cmd = ["lv2apply", "-i", str(src), "-o", str(dst)]
    for k, v in ctl.items():
        cmd += ["-c", k, str(v)]
    cmd.append(case["uri"])
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        cmd = [sys.executable, str(HERE / "lv2host.py"), case["uri"], str(src), str(dst), *[f"{k}={v}" for k, v in ctl.items()]]
        r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr[-300:])


def load(p):
    y = sf.read(p, dtype="float64")[0]
    return y if y.ndim == 1 else y[:, 0]


def band_db(x, lo, hi):
    sos = butter(6, [lo, hi], "bandpass", fs=FS, output="sos") if lo > 0 else butter(8, hi, "lowpass", fs=FS, output="sos")
    y = sosfilt(sos, x)
    fr = y[: len(y) // FR * FR].reshape(-1, FR)
    return 10 * np.log10((fr ** 2).mean(1) + 1e-20)


def labels(x):
    fr = x[: len(x) // FR * FR].reshape(-1, FR)
    act = 10 * np.log10((fr ** 2).mean(1) + 1e-20) > -50
    hi, lo = band_db(x, 4000, 12000), band_db(x, 0, 4000)
    sib = act & (hi > lo)
    return sib, act & ~sib


def score(x, y, sib, oth):
    x, y, lag = align(x, y, FS)
    n = min(len(sib), len(x) // FR)
    sib, oth = sib[:n], oth[:n]
    dh = band_db(y, 4500, 10000)[:n] - band_db(x, 4500, 10000)[:n]
    dl = band_db(y, 0, 4000)[:n] - band_db(x, 0, 4000)[:n]
    # power averages over frames, so a few deep frames are not hidden by many untouched ones
    pm = lambda d, m, xb: 10 * np.log10(np.sum(10 ** ((xb[m] + d[m]) / 10)) / np.sum(10 ** (xb[m] / 10)))
    xh, xl = band_db(x, 4500, 10000)[:n], band_db(x, 0, 4000)[:n]
    return {"sib_hf": -pm(dh, sib, xh), "sib_low": -pm(dl, sib, xl), "other_hf": -pm(dh, oth, xh),
            "lag_ms": lag / FS * 1e3}


def main():
    clean, work = Path(sys.argv[1]), Path(sys.argv[2])
    work.mkdir(parents=True, exist_ok=True)
    parts = []
    for f in sorted(clean.glob("*.wav"))[:4]:
        v, fs = sf.read(f, dtype="float64")
        assert fs == FS
        parts.append((v if v.ndim == 1 else v[:, 0])[: 20 * FS])
    x = np.concatenate(parts)
    x *= 10 ** (-3 / 20) / np.abs(x).max()
    src, quiet = work / "speech.wav", work / "speech_q.wav"
    sf.write(src, x.astype(np.float32), FS, subtype="FLOAT")
    sf.write(quiet, (x * 10 ** (-15 / 20)).astype(np.float32), FS, subtype="FLOAT")
    sib, oth = labels(x)
    print(f"sibilant frames {sib.mean() * 100:.1f} %, other active {oth.mean() * 100:.1f} %")
    res = {}
    only = sys.argv[3:]
    for name, c in CASES.items():
        if only and name not in only:
            continue
        lo, hi = c["lo"], c["hi"]
        dst = work / f"{name}.wav"
        best = None
        for _ in range(14):
            mid = np.sqrt(lo * hi) if c.get("log") else (lo + hi) / 2
            run(c, mid, src, dst)
            s = score(x, load(dst), sib, oth)
            best = (mid, s)
            # a higher threshold means less reduction for every processor here
            if s["sib_hf"] > 6:
                lo = mid
            else:
                hi = mid
            if abs(s["sib_hf"] - 6) < 0.1:
                break
        val, s = best
        run(c, val, quiet, work / f"{name}_q.wav")
        xq = x * 10 ** (-15 / 20)
        yq = load(work / f"{name}_q.wav")
        s["quiet_hf"] = score(xq, yq, sib, oth)["sib_hf"]
        s["setting"] = float(val)
        res[name] = s
        print(name, {k: round(v, 3) for k, v in s.items()}, flush=True)
    (work / "results.json").write_text(json.dumps(res, indent=1))
    print(f"\n{'case':16} {'sib_hf':>7} {'sib_low':>7} {'other_hf':>8} {'quiet_hf':>8} {'lag':>5}")
    for n, s in res.items():
        print(f"{n:16} {s['sib_hf']:7.2f} {s['sib_low']:7.2f} {s['other_hf']:8.2f} {s['quiet_hf']:8.2f} {s['lag_ms']:5.1f}")


if __name__ == "__main__":
    main()
