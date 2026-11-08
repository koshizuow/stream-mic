"""Peak limiter comparison: threshold -6 dBFS, release about 50 ms, lookahead 5 ms where there is one.

  uv run python limit_bench.py SPEECH_WAV WORK_DIR [CASE ...]

Signals (48 kHz):
- speech: SPEECH_WAV (clean DAPS voice) scaled so its peak is 0 dBFS, 6 dB over the threshold; speech12 peaks 6 dB higher
- burst:  1 kHz at -20 dBFS with a 500 ms jump to 0 dBFS (attack ahead of the peak, release afterwards)
- low:    100 Hz at 0 dBFS (distortion: a fast gain follows the waveform of a low note)
- isp:    12 kHz at 45 degrees phase, samples at -3 dBFS but the waveform peaks at 0 dBFS between them

Reports: latency, the highest sample peak and true peak (4x) over the threshold, the THD of the 100 Hz tone, and the
gain trajectory around the burst.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
from measure import align  # noqa: E402
from reflim import limit  # noqa: E402

FS = 48000
THR = -6.0

CASES = {
    "ref_5ms": {"ref": [5, 0, 50, False]},
    "ref_2ms_hold10": {"ref": [2, 10, 50, False]},
    "ref_2ms_tp": {"ref": [2, 10, 50, True]},
    # our Rust limiter (STREAM_MIC_CLI = path to stream-mic-cli): causal true peak, as in real time
    "rust_2ms_tp": {"cli": "ceil=-6,rel=50,look=2,hold=10,tp=1"},
    "rust_2ms": {"cli": "ceil=-6,rel=50,look=2,hold=10,tp=0"},
    "lsp_herm_thin": {"uri": "http://lsp-plug.in/plugins/lv2/limiter_mono",
                      "controls": {"th": 0.501187, "alr": 0, "boost": 0, "lk": 5, "at": 5, "rt": 50, "mode": 0}},
    "lsp_exp_thin": {"uri": "http://lsp-plug.in/plugins/lv2/limiter_mono",
                     "controls": {"th": 0.501187, "alr": 0, "boost": 0, "lk": 5, "at": 5, "rt": 50, "mode": 4}},
    "lsp_herm_thin_x4": {"uri": "http://lsp-plug.in/plugins/lv2/limiter_mono",
                         "controls": {"th": 0.501187, "alr": 0, "boost": 0, "lk": 5, "at": 5, "rt": 50, "mode": 0,
                                      "ovs": 16}},
    "calf": {"uri": "http://calf.sourceforge.net/plugins/Limiter",
             "controls": {"limit": 0.501187, "attack": 5, "release": 50, "asc": 0, "auto_level": 0}},
    "calf_asc": {"uri": "http://calf.sourceforge.net/plugins/Limiter",
                 "controls": {"limit": 0.501187, "attack": 5, "release": 50, "asc": 1, "auto_level": 0}},
    "zam_maximx2": {"uri": "urn:zamaudio:ZaMaximX2", "controls": {"thresh": -6, "ceil": -6, "rel": 50}},
    "mda": {"uri": "http://drobilla.net/plugins/mda/Limiter",
            "controls": {"thresh": 0.85, "output": 0.5, "knee": 0}},
}


def db(v):
    return 20 * np.log10(np.maximum(v, 1e-12))


def make_signals(speech, d):
    d.mkdir(parents=True, exist_ok=True)
    s, fs = sf.read(speech, dtype="float64")
    assert fs == FS
    s = s if s.ndim == 1 else s[:, 0]
    s = s[: 20 * FS] / np.abs(s[: 20 * FS]).max()
    t = np.arange(FS * 6) / FS
    burst = 0.1 * np.sin(2 * np.pi * 1000 * t)
    on = (t >= 2) & (t < 2.5)
    burst[on] *= 10
    sigs = {
        "speech": s,
        "speech12": s * 2.0,
        "burst": burst,
        "low": np.sin(2 * np.pi * 100 * np.arange(4 * FS) / FS),
        "isp": 10 ** (-3 / 20) / np.sin(np.pi / 4) * np.sin(2 * np.pi * 12000 * np.arange(4 * FS) / FS + np.pi / 4),
    }
    for k, v in sigs.items():
        sf.write(d / f"{k}.wav", v.astype(np.float32), FS, subtype="FLOAT")
    return list(sigs)


def run(name, case, src, dst):
    if "cli" in case:
        subprocess.run([os.environ["STREAM_MIC_CLI"], str(src), str(dst), "--lim", case["cli"]], check=True, capture_output=True)
        return
    if "ref" in case:
        look, hold, rel, tp = case["ref"]
        x, _ = sf.read(src, dtype="float64")
        y, _, _ = limit(x, FS, THR, look, hold, rel, tp)
        sf.write(dst, y.astype(np.float32), FS, subtype="FLOAT")
        return
    cmd = ["lv2apply", "-i", str(src), "-o", str(dst)]
    for k, v in case["controls"].items():
        cmd += ["-c", k, str(v)]
    cmd.append(case["uri"])
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        # lv2apply lacks the options feature and atom ports; our lilv host has them
        cmd = [sys.executable, str(HERE / "lv2host.py"), case["uri"], str(src), str(dst),
               *[f"{k}={v}" for k, v in case["controls"].items()]]
        r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(f"{name}: {r.stderr[-300:]}")


def load(p):
    x, _ = sf.read(p, dtype="float64")
    return x if x.ndim == 1 else x[:, 0]


def true_peak(y):
    """4x oversampled peak, skipping 0.25 s at each end (an abrupt start or stop rings in the resampler itself)."""
    up = resample_poly(y, 4, 1)
    return np.abs(up[FS: -FS]).max()


def thd(y):
    seg = y[2 * FS: 3 * FS]
    spec = np.abs(np.fft.rfft(seg * np.hanning(len(seg))))
    f = np.fft.rfftfreq(len(seg), 1 / FS)
    amp = lambda fr: spec[np.argmin(np.abs(f - fr)) - 3: np.argmin(np.abs(f - fr)) + 4].max()
    return 100 * np.sqrt(sum(amp(100 * k) ** 2 for k in range(2, 30))) / amp(100)


def burst_times(x, y):
    """Gain in dB with a 1 ms window: how early it starts to drop before the jump, how long it takes to recover."""
    n = FS // 1000
    g = db(np.sqrt(np.convolve(y ** 2, np.ones(n) / n, "same") / np.maximum(np.convolve(x ** 2, np.ones(n) / n, "same"), 1e-20)))
    t0, t1 = 2 * FS, int(2.5 * FS)
    pre = g[t0 - 20 * n: t0]
    early = np.flatnonzero(pre < -0.5)
    ahead = (len(pre) - early[0]) / n if len(early) else 0.0
    depth = np.median(g[t0 + 50 * n: t1 - 5 * n])
    post = g[t1: t1 + FS]
    rec = np.flatnonzero(post > depth * (1 - 0.632))
    rel = rec[0] / n if len(rec) else float("nan")
    rec90 = np.flatnonzero(post > depth * 0.1)
    return ahead, depth, rel, (rec90[0] / n if len(rec90) else float("nan"))


def main():
    speech, work = sys.argv[1], Path(sys.argv[2])
    sigs = make_signals(speech, work / "sig")
    rows = {}
    only = sys.argv[3:]
    for name, case in CASES.items():
        if only and name not in only:
            continue
        out = work / name
        out.mkdir(parents=True, exist_ok=True)
        r = {}
        for s in sigs:
            run(name, case, work / "sig" / f"{s}.wav", out / f"{s}.wav")
            x, y = load(work / "sig" / f"{s}.wav"), load(out / f"{s}.wav")
            xa, ya, lag = align(x, y, FS)
            if s == "speech":
                r["lag_ms"] = lag / FS * 1e3
            if s.startswith("speech") or s == "isp":
                r[f"{s}_sp"] = float(db(np.abs(ya).max()) - THR)
                r[f"{s}_tp"] = float(db(true_peak(ya)) - THR)
            if s == "speech":
                r["speech_gr_db"] = float(db(np.sqrt(np.mean(ya ** 2))) - db(np.sqrt(np.mean(xa ** 2))))
            if s == "low":
                r["thd_low"] = float(thd(ya))
            if s == "burst":
                r["ahead_ms"], r["depth_db"], r["rel63_ms"], r["rel90_ms"] = map(float, burst_times(xa, ya))
        rows[name] = r
        print(name, {k: round(v, 2) for k, v in r.items()}, flush=True)
    (work / "results.json").write_text(json.dumps(rows, indent=1))
    print(f"\n{'case':18} {'lat':>5} {'sp':>6} {'tp':>6} {'sp+12':>6} {'tp+12':>6} {'isp tp':>6} {'THD100':>7} "
          f"{'ahead':>5} {'depth':>6} {'rel63':>6} {'rel90':>6}")
    for n, r in rows.items():
        print(f"{n:18} {r['lag_ms']:5.1f} {r['speech_sp']:6.2f} {r['speech_tp']:6.2f} {r['speech12_sp']:6.2f} "
              f"{r['speech12_tp']:6.2f} {r['isp_tp']:6.2f} {r['thd_low']:7.2f} {r['ahead_ms']:5.1f} "
              f"{r['depth_db']:6.2f} {r['rel63_ms']:6.1f} {r['rel90_ms']:6.1f}")


if __name__ == "__main__":
    main()
