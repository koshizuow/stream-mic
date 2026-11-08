"""Reference compressor from Giannoulis, Massberg, Reiss (JAES 2012), the design they recommend:
feed-forward, detector in the log domain after the gain computer, smooth decoupled peak detector.

Used to check the measurement tools: its parameters are known, so measure.py must recover them.

Usage: uv run python refcomp.py IN.wav OUT.wav T R W ATTACK_MS RELEASE_MS [MAKEUP_DB]
"""
import sys

import numpy as np
import soundfile as sf


def gain_computer(xg, T, R, W):
    yg = xg.copy()
    over = 2 * (xg - T)
    knee = np.abs(over) <= W
    above = over > W
    if W > 0:
        yg[knee] = xg[knee] + (1 / R - 1) * (xg[knee] - T + W / 2) ** 2 / (2 * W)
    yg[above] = T + (xg[above] - T) / R
    return yg


def smooth_decoupled(xl, fs, attack_ms, release_ms):
    aa = np.exp(-1 / (attack_ms * 1e-3 * fs))
    ar = np.exp(-1 / (release_ms * 1e-3 * fs))
    y1 = 0.0
    yl = 0.0
    out = np.empty_like(xl)
    for n, x in enumerate(xl):
        y1 = max(x, ar * y1 + (1 - ar) * x)
        yl = aa * yl + (1 - aa) * y1
        out[n] = yl
    return out


def compress(x, fs, T, R, W, attack_ms, release_ms, makeup_db=0.0, sc_hp=0.0):
    from refexp import sidechain  # Butterworth high pass on the detector input, 0 = off
    xg = 20 * np.log10(np.abs(sidechain(x, fs, sc_hp)) + 1e-12)
    yg = gain_computer(xg, T, R, W)
    xl = xg - yg
    yl = smooth_decoupled(xl, fs, attack_ms, release_ms)
    c = 10 ** ((-yl + makeup_db) / 20)
    return x * c, -yl


def main():
    src, dst = sys.argv[1], sys.argv[2]
    T, R, W, at, rt = (float(v) for v in sys.argv[3:8])
    mk = float(sys.argv[8]) if len(sys.argv) > 8 else 0.0
    x, fs = sf.read(src, dtype="float64")
    y, _ = compress(x, fs, T, R, W, at, rt, mk)
    sf.write(dst, y.astype(np.float32), fs, subtype="FLOAT")


if __name__ == "__main__":
    main()
