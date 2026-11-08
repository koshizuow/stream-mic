"""Reference downward expander: the detector only says how loud it is; opening and closing speeds are set
on the gain, in dB per time, with a hold (measurements.md: plugin expanders close later the louder you
spoke). Same algorithm as crates/dsp/src/dynamics.rs Expander, written out plainly.

  env[n] = max(|sc[n]|, a_det * env[n-1])                       peak level, decays with det_ms
  t[n]   = expander curve(20 log10 env): (r - 1)(L - T) below T, quadratic knee W, floor -range
  g      : t >= g -> rise at range/att per ms (and restart the hold); else wait out the hold,
           then fall at range/rel per ms
  y      = x * 10^(g/20)

Usage: uv run python refexp.py IN OUT T R RANGE [KNEE ATT HOLD REL DET SC_HP]
"""
import sys

import numpy as np
import soundfile as sf
from scipy import signal


def curve(L, T, r, w, rng):
    d = L - T
    if 2 * d > w:
        g = 0.0
    elif 2 * d >= -w and w > 0:
        g = -(r - 1) * (d - w / 2) ** 2 / (2 * w)
    else:
        g = (r - 1) * d
    return max(g, -rng)


def sidechain(x, fs, hp):
    if hp <= 0:
        return x
    # same as the SVF high pass: bilinear Butterworth with the corner prewarped
    wc = 2 * fs * np.tan(np.pi * hp / fs)
    b, a = signal.bilinear([1, 0, 0], [1, wc * np.sqrt(2), wc * wc], fs)
    return signal.lfilter(b, a, x)


def expand(x, fs, T, r, rng, knee=10.0, att=3.0, hold=80.0, rel=200.0, det=10.0, sc_hp=100.0):
    sc = sidechain(x, fs, sc_hp)
    ad = np.exp(-1 / (det * 1e-3 * fs)) if det > 0 else 0.0
    up = rng / (max(att, 0.01) * 1e-3 * fs)
    down = rng / (max(rel, 0.01) * 1e-3 * fs)
    hold_n = int(round(hold * 1e-3 * fs))
    env = 0.0
    g = 0.0
    h = 0
    gains = np.empty(len(x))
    for n, v in enumerate(sc):
        env = max(abs(v), ad * env)
        t = curve(20 * np.log10(env + 1e-12), T, r, knee, rng)
        if t >= g:
            g = min(t, g + up)
            h = hold_n
        elif h > 0:
            h -= 1
        else:
            g = max(t, g - down)
        gains[n] = g
    return x * 10 ** (gains / 20), gains


def main():
    src, dst = sys.argv[1], sys.argv[2]
    vals = [float(v) for v in sys.argv[3:]]
    x, fs = sf.read(src, dtype="float64")
    if x.ndim > 1:
        x = x[:, 0]
    y, _ = expand(x, fs, *vals)
    sf.write(dst, y.astype(np.float32), fs, subtype="FLOAT")


if __name__ == "__main__":
    main()
