"""Check the Rust reverb against reffdn.py (build-plan.md step 3): mono and stereo, with and without a
backing track to follow, identical at block sizes 1, 128 and 4096.

Usage: uv run python verb_check.py CLI WORKDIR SPEECH.wav
"""
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

from reffdn import follow_mix, reverb

FS = 48000
P = dict(t60=1.2, pre=10, lowcut=250, damp=7000, mix=0.3, mixdry=0.1, follow=1.5)


def run(cli, inp, out, block, music=None, stereo=False):
    cmd = [cli, str(inp), str(out), "--block", str(block), "--verb", ",".join(f"{k}={v}" for k, v in P.items())]
    if music:
        cmd += ["--music", str(music)]
    if stereo:
        cmd.append("--stereo")
    subprocess.run(cmd, check=True, capture_output=True)
    return sf.read(out, dtype="float64")[0]


def err_db(y, ref):
    return 20 * np.log10(np.sqrt(np.mean((y - ref) ** 2)) / np.sqrt(np.mean(ref ** 2)) + 1e-30)


def main():
    cli, work, speech = sys.argv[1], Path(sys.argv[2]), sys.argv[3]
    work.mkdir(parents=True, exist_ok=True)
    s, fs = sf.read(speech, dtype="float64")
    s = (s if s.ndim == 1 else s[:, 0])[: 8 * FS]
    s = s / np.abs(s).max() * 0.5
    inp = work / "verb_in.wav"
    sf.write(inp, s.astype(np.float32), FS, subtype="FLOAT")
    rng = np.random.default_rng(3)
    music = rng.standard_normal(len(s)) * 0.05
    music[3 * FS: 6 * FS] = 0  # music stops for 3 s
    mus = work / "verb_music.wav"
    sf.write(mus, music.astype(np.float32), FS, subtype="FLOAT")
    yl, yr = reverb(s, FS, P["t60"], P["pre"], P["lowcut"], P["damp"])
    mix_m = follow_mix(music, len(s), FS, P["mix"], P["mixdry"], P["follow"])
    want = {
        "mono": s + P["mix"] * (yl + yr) / 2,
        "stereo": np.stack([s + P["mix"] * yl, s + P["mix"] * yr], 1),
        "music": s + mix_m * (yl + yr) / 2,
    }
    ok = True
    for name, ref in want.items():
        outs = [run(cli, inp, work / f"verb_{name}_{b}.wav", b, mus if name == "music" else None, name == "stereo")
                for b in (1, 128, 4096)]
        same = all(np.array_equal(outs[0], o) for o in outs[1:])
        e = err_db(outs[1], ref)
        good = e < -100 and same
        ok &= good
        print(f"{name:7s} error {e:7.1f} dB  blocks identical {same}" + ("" if good else "  <-- FAIL"))
    print(f"mix with music: {mix_m[2 * FS]:.3f} at 2 s, {mix_m[int(5.9 * FS)]:.3f} at 5.9 s, {mix_m[-1]:.3f} at the end")
    print("PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
