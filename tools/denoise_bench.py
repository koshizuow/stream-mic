"""Denoiser comparison on DAPS (run on a Linux test PC).

  uv run python denoise_bench.py ROOT

ROOT holds data/{clean,device,full} (48 kHz mono, see measurements.md), rnnoise/ (built), df/ (deep-filter binary and
model tars) and dnsmos/sig_bak_ovr.onnx. It writes ROOT/work/ and prints tables.

Two kinds of test:
- mixes: clean studio speech plus real room tone at 0, 10 and 20 dB SNR. The room tone is cut from the quietest
  20 ms frames of the device recordings of that room (the tablet or phone in the same room, no speech), so the noise
  is what a real room and a cheap microphone sound like. Scored against the clean speech: PESQ (wide band), STOI,
  SI-SDR, and DNSMOS (no reference).
- device: the real tablet and phone recordings, scored with DNSMOS only (there is no exact clean reference: room echo).

Each denoiser runs on one core (taskset -c 0) and its real-time factor is timed. Latency: the offset of the output
against the input by cross-correlation (offline tools may already compensate it, see the notes).
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import onnxruntime as ort
import soundfile as sf
from pesq import pesq
from pystoi import stoi
from scipy.signal import resample_poly

ROOT = Path(sys.argv[1])
DATA, WORK = ROOT / "data", ROOT / "work"
FS = 48000
ROOMS = ["ipad_livingroom1", "iphone_balcony1", "ipad_confroom1", "ipad_office1"]
SPEAKERS = ["f1", "f3", "m1", "m3"]
SNRS = [0, 10, 20]
DF = ROOT / "df" / "deep-filter-0.5.6-x86_64-unknown-linux-musl"


def load(p):
    x, fs = sf.read(p, dtype="float32")
    assert fs == FS, p
    return x if x.ndim == 1 else x[:, 0]


def room_tone(room):
    """The quietest 10 % of 20 ms frames of every recording in this room, joined with 5 ms crossfades."""
    n, fade = int(0.02 * FS), int(0.005 * FS)
    w = np.linspace(0, 1, fade, dtype=np.float32)
    out = []
    for sp in SPEAKERS:
        x = load(DATA / "full" / f"{sp}_{room}.wav")
        fr = x[: len(x) // n * n].reshape(-1, n)
        e = (fr ** 2).mean(1)
        keep = np.sort(np.flatnonzero(e <= np.percentile(e, 10)))
        # runs of neighbouring quiet frames stay whole, so the tone is not chopped every 20 ms
        runs, start = [], keep[0]
        for a, b in zip(keep[:-1], keep[1:]):
            if b != a + 1:
                runs.append((start, a)); start = b
        runs.append((start, keep[-1]))
        for a, b in runs:
            if b - a >= 2:
                out.append(x[a * n:(b + 1) * n])
    y = out[0]
    for seg in out[1:]:
        y = np.concatenate([y[:-fade], y[-fade:] * (1 - w) + seg[:fade] * w, seg[fade:]])
    return y


def active_rms(x):
    n = int(0.02 * FS)
    fr = x[: len(x) // n * n].reshape(-1, n)
    e = (fr ** 2).mean(1)
    return np.sqrt(e[e > e.max() * 1e-3].mean())


def make_mixes():
    (WORK / "mix").mkdir(parents=True, exist_ok=True)
    for room in ROOMS:
        tone = room_tone(room)
        sf.write(WORK / f"tone_{room}.wav", tone, FS)
        for sp in SPEAKERS:
            s = load(DATA / "clean" / f"{sp}.wav")
            nz = np.resize(tone, len(s))
            for snr in SNRS:
                g = active_rms(s) / (np.sqrt((nz ** 2).mean()) * 10 ** (snr / 20))
                m = s + g * nz
                m *= min(1.0, 0.9 / np.abs(m).max())
                sf.write(WORK / "mix" / f"{sp}_{room}_{snr}.wav", m.astype(np.float32), FS, subtype="FLOAT")
    print("room tone seconds:", {r: round(len(load(WORK / f"tone_{r}.wav")) / FS, 1) for r in ROOMS})


def run_rnnoise(src, dst):
    raw_in, raw_out = dst.with_suffix(".in.raw"), dst.with_suffix(".out.raw")
    x = load(src)
    (np.clip(x, -1, 1) * 32767).astype("<i2").tofile(raw_in)
    t = time.perf_counter()
    subprocess.run(["taskset", "-c", "0", str(ROOT / "rnnoise/examples/rnnoise_demo"), str(raw_in), str(raw_out)], check=True,
                   capture_output=True)
    dt = time.perf_counter() - t
    y = np.fromfile(raw_out, "<i2").astype(np.float32) / 32768
    sf.write(dst, y, FS, subtype="FLOAT")
    raw_in.unlink(); raw_out.unlink()
    return dt


def run_df(model, extra):
    def run(src, dst):
        tmp = dst.parent / "_df"
        tmp.mkdir(exist_ok=True)
        t = time.perf_counter()
        subprocess.run(["taskset", "-c", "0", str(DF), "-m", str(ROOT / "df" / model), *extra, "-o", str(tmp), str(src)],
                       check=True, capture_output=True, env={"RAYON_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"})
        dt = time.perf_counter() - t
        (tmp / src.name).rename(dst)
        return dt
    return run


def run_ours(limit, thresh=None):
    """Our Rust stage (STREAM_MIC_CLI), the same DF3 low latency model embedded through libDF. thresh: libDF's
    processing thresholds "min,erb,df" (default: the deep-filter program's -15,35,35)."""
    def run(src, dst):
        t = time.perf_counter()
        env = {"RAYON_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"}
        if thresh:
            env["STREAM_MIC_DF_THRESH"] = thresh
        subprocess.run(["taskset", "-c", "0", os.environ["STREAM_MIC_CLI"], str(src), str(dst), "--denoise", f"limit={limit}"],
                       check=True, capture_output=True, env=env)
        return time.perf_counter() - t
    return run


SYSTEMS = {
    "noisy": None,
    "rnnoise": run_rnnoise,
    "df3": run_df("DeepFilterNet3_onnx.tar.gz", []),
    "df3_ll": run_df("DeepFilterNet3_ll_onnx.tar.gz", []),
    "df3_lim12": run_df("DeepFilterNet3_onnx.tar.gz", ["-a", "12"]),
    "ours_ll": run_ours(100),
    "ours_ll_lim24": run_ours(24),
    # libDF's own defaults (the LADSPA plugin's): skips the DF stage sooner, about 30 % less CPU
    "ours_ll_libdef": run_ours(100, "-10,30,20"),
}
if os.environ.get("ONLY"):  # e.g. ONLY=noisy,df3_ll,ours_ll
    SYSTEMS = {k: v for k, v in SYSTEMS.items() if k in os.environ["ONLY"].split(",")}


def offset(x, y, max_ms=100):
    n = int(max_ms / 1e3 * FS)
    a, b = x[FS: FS * 6], y[FS: FS * 6]
    c = [np.dot(a[n:-n], b[n + k: len(b) - n + k]) for k in range(-n, n + 1, 1)]
    return (int(np.argmax(c)) - n) / FS * 1e3


class DNSMOS:
    """Microsoft DNSMOS P.835 (sig_bak_ovr.onnx): 9.01 s windows at 16 kHz, polynomial fit as in their script."""
    def __init__(self, path):
        self.s = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])

    def __call__(self, x48):
        x = resample_poly(x48, 1, 3).astype(np.float32)
        L = int(9.01 * 16000)
        if len(x) < L:
            x = np.resize(x, L)
        res = []
        for st in range(0, len(x) - L + 1, 16000):
            o = self.s.run(None, {"input_1": x[st:st + L][None, :]})[0][0]
            sig, bak, ovr = o
            p_sig = np.poly1d([-0.08397278, 1.22083953, 0.0052439])(sig)
            p_bak = np.poly1d([-0.13166888, 1.60915514, -0.39604546])(bak)
            p_ovr = np.poly1d([-0.06766283, 1.11546468, 0.04602535])(ovr)
            res.append((p_sig, p_bak, p_ovr))
        return np.mean(res, 0)


def si_sdr(ref, est):
    n = min(len(ref), len(est)); ref, est = ref[:n] - ref[:n].mean(), est[:n] - est[:n].mean()
    a = np.dot(est, ref) / np.dot(ref, ref)
    return 10 * np.log10(np.sum((a * ref) ** 2) / np.sum((a * ref - est) ** 2))


def main():
    if not (WORK / "mix").exists():
        make_mixes()
    dns = DNSMOS(ROOT / "dnsmos" / "sig_bak_ovr.onnx")
    rows, times = [], {}
    sets = {"mix": sorted((WORK / "mix").glob("*.wav")), "device": sorted((DATA / "device").glob("*.wav"))}
    for name, fn in SYSTEMS.items():
        for kind, files in sets.items():
            out = WORK / name / kind
            out.mkdir(parents=True, exist_ok=True)
            for f in files:
                dst = out / f.name
                if fn and not dst.exists():
                    dt = fn(f, dst)
                    times.setdefault(name, []).append(dt / (len(load(f)) / FS))
                y = load(dst if fn else f)
                x = load(f)
                lag = offset(x, y) if fn else 0.0
                k = int(round(lag / 1e3 * FS))
                ya = y[k:] if k > 0 else np.concatenate([np.zeros(-k, np.float32), y])
                r = {"sys": name, "kind": kind, "file": f.stem, "lag_ms": lag}
                r["sig"], r["bak"], r["ovr"] = map(float, dns(ya))
                if kind == "mix":
                    sp = f.stem.split("_")[0]
                    ref = load(DATA / "clean" / f"{sp}.wav")
                    n = min(len(ref), len(ya))
                    r["snr"] = int(f.stem.split("_")[-1])
                    r["pesq"] = float(pesq(16000, resample_poly(ref[:n], 1, 3), resample_poly(ya[:n], 1, 3), "wb"))
                    r["stoi"] = float(stoi(ref[:n], ya[:n], FS, extended=False))
                    r["sisdr"] = float(si_sdr(ref, ya))
                rows.append(r)
            print("done", name, kind, flush=True)
    (WORK / "results.json").write_text(json.dumps({"rows": rows, "rtf": {k: float(np.median(v)) for k, v in times.items()}}, indent=1))
    summarize(rows, times)


def summarize(rows, times):
    import collections
    print("\nmixes (clean speech + real room tone), mean over 4 speakers x 4 rooms")
    print(f"{'system':10} {'SNR':>4} {'PESQ':>5} {'STOI':>5} {'SI-SDR':>6} {'SIG':>5} {'BAK':>5} {'OVRL':>5}")
    g = collections.defaultdict(list)
    for r in rows:
        if r["kind"] == "mix":
            g[(r["sys"], r["snr"])].append(r)
    for (s, snr), rs in sorted(g.items(), key=lambda kv: (kv[0][1], list(SYSTEMS).index(kv[0][0]))):
        m = lambda k: np.mean([r[k] for r in rs])
        print(f"{s:10} {snr:4} {m('pesq'):5.2f} {m('stoi'):5.3f} {m('sisdr'):6.1f} {m('sig'):5.2f} {m('bak'):5.2f} {m('ovr'):5.2f}")
    print("\nreal device recordings, DNSMOS, mean per room over 4 speakers")
    g = collections.defaultdict(list)
    for r in rows:
        if r["kind"] == "device":
            g[(r["sys"], "_".join(r["file"].split("_")[1:]))].append(r)
    for (s, room), rs in sorted(g.items(), key=lambda kv: (kv[0][1], list(SYSTEMS).index(kv[0][0]))):
        m = lambda k: np.mean([r[k] for r in rs])
        print(f"{room:17} {s:10} SIG {m('sig'):4.2f} BAK {m('bak'):4.2f} OVRL {m('ovr'):4.2f}")
    print("\nreal-time factor on one core (lower is faster), output offset against input")
    for s in SYSTEMS:
        if s in times:
            lags = [r["lag_ms"] for r in rows if r["sys"] == s]
            print(f"{s:10} RTF {np.median(times[s]):.3f}  offset median {np.median(lags):6.1f} ms")


if __name__ == "__main__":
    main()
