"""Test signals for measuring processors (48 kHz mono float32 WAV).

ramp:    1 kHz sine rising -60 to 0 dBFS at 1.5 dB/s (static curve; Thompson and Heilemann 2026 use 1.6 dB/s)
bursts:  1 kHz sine at -40 dBFS with jumps to -10 dBFS lasting 50 ms, 500 ms and 3 s (attack, release, program-dependent release)
noise:   pink noise at -20 dBFS RMS, 20 s (frequency response by cross spectrum)
impulse: one sample at -6 dBFS then 6 s of silence (reverb decay)
lowsine: 100 Hz sine at -6 dBFS, 4 s (distortion from fast attack)

Usage: uv run python signals.py OUT_DIR
"""
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

FS = 48000


def db(x):
    return 10 ** (x / 20)


def ramp(f=1000.0, start=-60.0, stop=0.0, rate=1.5, pad=1.0):
    dur = (stop - start) / rate
    t = np.arange(int(dur * FS)) / FS
    level = start + rate * t
    sig = db(level) * np.sin(2 * np.pi * f * t)
    z = np.zeros(int(pad * FS))
    return np.concatenate([z, sig, z])


def bursts(f=1000.0, low=-40.0, high=-10.0, lengths=(0.05, 0.5, 3.0), gap=3.0):
    parts = []
    phase = 0.0
    def tone(level, dur):
        nonlocal phase
        n = int(dur * FS)
        t = np.arange(n) / FS
        out = db(level) * np.sin(2 * np.pi * f * t + phase)
        phase += 2 * np.pi * f * n / FS
        return out
    parts.append(tone(low, gap))
    for length in lengths:
        parts.append(tone(high, length))
        parts.append(tone(low, gap))
    return np.concatenate(parts)


def pink(dur=20.0, rms_db=-20.0, seed=1):
    rng = np.random.default_rng(seed)
    n = int(dur * FS)
    spec = np.fft.rfft(rng.standard_normal(n))
    freqs = np.fft.rfftfreq(n, 1 / FS)
    freqs[0] = freqs[1]
    spec /= np.sqrt(freqs)
    x = np.fft.irfft(spec, n)
    return x / np.sqrt(np.mean(x ** 2)) * db(rms_db)


def impulse(level=-6.0, tail=6.0):
    x = np.zeros(int(tail * FS) + 1)
    x[0] = db(level)
    return np.concatenate([np.zeros(FS // 10), x])


def lowsine(f=100.0, level=-6.0, dur=4.0):
    t = np.arange(int(dur * FS)) / FS
    return db(level) * np.sin(2 * np.pi * f * t)


SIGNALS = {"ramp": ramp, "bursts": bursts, "noise": pink, "impulse": impulse, "lowsine": lowsine,
           # gates and expanders: the quiet part sits well under a -40 dB threshold, as room noise between words would
           "gatebursts": lambda: bursts(low=-55.0)}


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    for name, make in SIGNALS.items():
        x = make().astype(np.float32)
        sf.write(out / f"{name}.wav", x, FS, subtype="FLOAT")
        print(f"{name}.wav {len(x) / FS:.1f} s peak {20 * np.log10(np.max(np.abs(x)) + 1e-12):.1f} dBFS")


if __name__ == "__main__":
    main()
