"""T30 of an impulse response per octave band (Schroeder backward integration, -5 to -35 dB).

Usage: uv run python band_t60.py IR.wav [SKIP_S]
"""
import sys

import numpy as np
import soundfile as sf
from scipy import signal

h, fs = sf.read(sys.argv[1], dtype="float64")
h = h.mean(1) if h.ndim > 1 else h
skip = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0
h = h[int(skip * fs):]
for fc in (250, 500, 1000, 2000, 4000, 8000):
    sos = signal.butter(4, [fc / np.sqrt(2), fc * np.sqrt(2)], "bandpass", fs=fs, output="sos")
    y = signal.sosfilt(sos, h)
    e = np.cumsum(y[::-1] ** 2)[::-1]
    d = 10 * np.log10(e / e[0] + 1e-30)
    i5, i35 = np.argmax(d < -5), np.argmax(d < -35)
    t = np.arange(len(d)) / fs
    k = np.polyfit(t[i5:i35], d[i5:i35], 1)[0]
    print(f"{fc:5d} Hz T30 {-60 / k:5.2f} s")
