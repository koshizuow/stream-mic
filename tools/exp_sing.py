"""Expander settings on singing (build-plan.md step 2): what happens to note tails and to the noise
between phrases.

Mixes VocalSet singers (peak -6 dBFS) with real room tone at 20 and 30 dB SNR. The threshold is set
the way auto setup will (loudness.md): 8 dB above the room tone's detector level. Runs the Rust
expander (stream-mic-cli, bit-identical to refexp.py per dyn_check.py); the gain per frame is the
output to input energy ratio.

Frames are 50 ms, classified by the clean singing's level relative to its loudest frame:
  sung   within 30 dB    should be untouched
  tails  30 to 50 dB down (note decays, soft consonants, breaths): what a too eager expander eats
  gaps   more than 60 dB down (between phrases): how much the room tone is lowered

Usage: uv run python exp_sing.py CLI SING_DIR TONE_DIR WORKDIR
"""
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

import subprocess

from refexp import sidechain

FS = 48000
N = FS // 20
ROOMS = ["ipad_office1", "ipad_livingroom1"]
CONFIGS = {
    "talk (2.5, 12 dB, hold 80, rel 200)": dict(r=2.5, rng=12, hold=80, rel=200),
    "sing preset (1.7, 8 dB, hold 150, rel 600)": dict(r=1.7, rng=8, hold=150, rel=600),
    "sing slower (1.7, 8 dB, hold 300, rel 1000)": dict(r=1.7, rng=8, hold=300, rel=1000),
    "sing deeper (2.0, 10 dB, hold 150, rel 600)": dict(r=2.0, rng=10, hold=150, rel=600),
    # threshold 14 dB over the room tone instead of 8
    "talk, thr +14": dict(r=2.5, rng=12, hold=80, rel=200, off=14),
    "sing preset, thr +14": dict(r=1.7, rng=8, hold=150, rel=600, off=14),
    "sing slower, thr +14": dict(r=1.7, rng=8, hold=300, rel=1000, off=14),
    "sing 2.0 10 dB hold 300 rel 1000, thr +14": dict(r=2.0, rng=10, hold=300, rel=1000, off=14),
}


def load(p):
    x, fs = sf.read(p, dtype="float64")
    assert fs == FS
    return x if x.ndim == 1 else x[:, 0]


def frames(x):
    return x[: len(x) // N * N].reshape(-1, N)


def det_level(x):
    """Median of the expander's own detector on this signal, in dB."""
    sc = np.abs(sidechain(x, FS, 100.0))
    a = np.exp(-1 / (10e-3 * FS))
    env = np.empty_like(sc)
    e = 0.0
    for i, v in enumerate(sc):
        e = max(v, a * e)
        env[i] = e
    return float(np.median(20 * np.log10(env[FS:] + 1e-12)))


def main():
    cli, sing, tones, work = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4])
    work.mkdir(parents=True, exist_ok=True)
    rows = {k: [] for k in CONFIGS}
    for f in sorted(sing.glob("*.wav")):
        s = load(f)
        s = s[: len(s) // N * N]
        s = s / np.abs(s).max() * 10 ** (-6 / 20)
        lv = 10 * np.log10((frames(s) ** 2).mean(1) + 1e-20)
        lv -= lv.max()
        cls = {"sung": lv > -30, "tails": (lv <= -30) & (lv > -50), "gaps": lv < -60}
        active = s[np.repeat(lv > -30, N)[: len(s)]]
        for room in ROOMS:
            tone = np.resize(load(tones / f"tone_{room}.wav"), len(s))
            for snr in (20, 30):
                nz = tone / np.sqrt((tone ** 2).mean()) * np.sqrt((active ** 2).mean()) * 10 ** (-snr / 20)
                floor = det_level(nz[: 10 * FS])
                m = s + nz
                sf.write(work / "in.wav", m.astype(np.float32), FS, subtype="FLOAT")
                ein = (frames(sf.read(work / "in.wav", dtype="float64")[0]) ** 2).sum(1)
                for name, c in CONFIGS.items():
                    thr = floor + c.get("off", 8)
                    arg = f"thr={thr},ratio={c['r']},range={c['rng']},knee=10,att=3,hold={c['hold']},rel={c['rel']},det=10,schp=100"
                    subprocess.run([cli, str(work / "in.wav"), str(work / "out.wav"), "--exp", arg], check=True, capture_output=True)
                    eout = (frames(sf.read(work / "out.wav", dtype="float64")[0]) ** 2).sum(1)
                    gf = 10 * np.log10((eout + 1e-30) / (ein + 1e-30))
                    r = {"thr": thr}
                    lens = np.diff(np.flatnonzero(np.diff(np.concatenate([[0], cls["gaps"].astype(int), [0]]))))[::2]
                    r["gap_s"] = float(np.median(lens) * N / FS) if len(lens) else np.nan
                    for k, mask in cls.items():
                        mask = mask[: len(gf)]
                        r[k + "_med"] = float(np.median(gf[mask])) if mask.any() else np.nan
                        r[k + "_3db"] = float((gf[mask] < -3).mean() * 100) if mask.any() else np.nan
                        r[k + "_1db"] = float((gf[mask] < -1).mean() * 100) if mask.any() else np.nan
                    rows[name].append(r)
                print(f"{f.stem} {room} {snr} dB: room tone detector {floor:.1f} dBFS", flush=True)
    print("median gap length", np.nanmean([r["gap_s"] for r in next(iter(rows.values()))]), "s")
    print(f"\n{'setting':46} {'sung <-1dB %':>12} {'tails med':>9} {'tails <-3dB %':>13} {'gaps med':>8} {'gaps <-3dB %':>12}")
    for name, rs in rows.items():
        m = lambda k: np.nanmean([r[k] for r in rs])
        print(f"{name:46} {m('sung_1db'):12.1f} {m('tails_med'):9.2f} {m('tails_3db'):13.1f} {m('gaps_med'):8.2f} {m('gaps_3db'):12.1f}")


if __name__ == "__main__":
    main()
