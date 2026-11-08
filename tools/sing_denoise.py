"""Denoisers on singing (run on a Linux test PC, after denoise_bench.py made the room tones).

  uv run python sing_denoise.py ROOT SING_DIR

SING_DIR: clean 48 kHz singing (VocalSet set: f1 f6 m3 m9). Each voice gets the four DAPS room tones at 10 and
20 dB SNR. PESQ and DNSMOS are trained on speech, so singing is scored with:
- SI-SDR against the clean voice
- held notes: on frames where the clean voice is loud (within 30 dB of its peak), the output level minus the clean
  level, 50 ms frames. "drop3" is the share of those frames more than 3 dB too quiet (a denoiser that mistakes a
  long note or vibrato for noise pumps it down); "hf" is the same difference above 5 kHz (dulling of the top end).
"""
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import butter, sosfilt

import denoise_bench as db  # noqa: E402  (reads ROOT from argv[1])

SING = Path(sys.argv[2])
OUT = db.WORK / "sing"
SYS = {k: db.SYSTEMS[k] for k in ["noisy", "rnnoise", "df3", "df3_ll", "df3_lim12"]}
N = int(0.05 * db.FS)


def frames_db(x):
    fr = x[: len(x) // N * N].reshape(-1, N)
    return 10 * np.log10((fr ** 2).mean(1) + 1e-20)


def main():
    (OUT / "mix").mkdir(parents=True, exist_ok=True)
    hp = butter(6, 5000, "highpass", fs=db.FS, output="sos")
    for f in sorted(SING.glob("*.wav")):
        s = db.load(f)
        for room in db.ROOMS:
            tone = db.load(db.WORK / f"tone_{room}.wav")
            nz = np.resize(tone, len(s))
            for snr in (10, 20):
                g = db.active_rms(s) / (np.sqrt((nz ** 2).mean()) * 10 ** (snr / 20))
                m = s + g * nz
                k = min(1.0, 0.9 / np.abs(m).max())
                sf.write(OUT / "mix" / f"{f.stem}_{room}_{snr}.wav", (m * k).astype(np.float32), db.FS, subtype="FLOAT")
                sf.write(OUT / "mix" / f"{f.stem}_{room}_{snr}.ref.wav", (s * k).astype(np.float32), db.FS, subtype="FLOAT")
    rows = []
    mixes = sorted(p for p in (OUT / "mix").glob("*.wav") if not p.name.endswith(".ref.wav"))
    for name, fn in SYS.items():
        (OUT / name).mkdir(exist_ok=True)
        for mx in mixes:
            dst = OUT / name / mx.name
            if fn and not dst.exists():
                fn(mx, dst)
            x, y = db.load(mx), db.load(dst if fn else mx)
            ref = db.load(mx.with_suffix(".ref.wav"))
            lag = db.offset(x, y) if fn else 0.0
            k = int(round(lag / 1e3 * db.FS))
            ya = y[k:] if k > 0 else np.concatenate([np.zeros(-k, np.float32), y])
            n = min(len(ref), len(ya))
            r, ya = ref[:n], ya[:n]
            lr, ly = frames_db(r), frames_db(ya)
            loud = lr > lr.max() - 30
            d = (ly - lr)[loud]
            dh = (frames_db(sosfilt(hp, ya)) - frames_db(sosfilt(hp, r)))[loud]
            rows.append({"sys": name, "snr": int(mx.stem.split("_")[-1]), "sisdr": float(db.si_sdr(r, ya)),
                         "drop3": float((d < -3).mean() * 100), "med": float(np.median(d)), "hf": float(np.median(dh))})
        print("done", name, flush=True)
    print(f"\n{'system':10} {'SNR':>4} {'SI-SDR':>7} {'drop3 %':>8} {'median dB':>9} {'>5k dB':>7}")
    for snr in (10, 20):
        for name in SYS:
            rs = [r for r in rows if r["sys"] == name and r["snr"] == snr]
            m = lambda k: np.mean([r[k] for r in rs])
            print(f"{name:10} {snr:4} {m('sisdr'):7.1f} {m('drop3'):8.1f} {m('med'):9.2f} {m('hf'):7.2f}")


if __name__ == "__main__":
    main()
