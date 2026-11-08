"""Reference plate reverb after Dattorro 1997 (JAES, "Effect Design Part 1", figure 1 and tables 1, 2),
plus what a live voice needs (design/dsp/reverb.md): pre-delay, a low cut on what goes into the reverb,
and the decay set as T60 in seconds.

Signal path (mono in, stereo wet out):
  x -> pre-delay -> low cut (2nd order high pass) -> 4 input diffusion allpasses (142, 107, 379, 277)
    -> figure-8 tank: each half = modulated allpass (-dd1) -> delay -> damping low pass -> *decay
                      -> allpass (dd2) -> delay -> *decay -> into the other half
    -> output taps (table 2)

Delay lengths are Dattorro's (at 29761 Hz) scaled to the sample rate. decay is set so a trip round the
tank (four multiplications) loses loop_seconds * 60 / T60 dB; measure.py decay checks the real T60.
Modulation: the two tank input allpasses swing EXC samples (scaled) at 1 Hz, in quadrature, read with
linear interpolation.

Same algorithm as crates/dsp/src/reverb.rs.

Usage: uv run python refverb.py IN OUT.wav T60 PRE_MS LOWCUT_HZ DAMP_HZ MIX [stereo]
(OUT is dry plus MIX times the wet; mono output is the average of the two wet sides)
"""
import sys

import numpy as np
import soundfile as sf
from scipy import signal

SR0 = 29761.0
IN_AP = [(142, 0.75), (107, 0.75), (379, 0.625), (277, 0.625)]
L_AP1, L_D1, L_AP2, L_D2 = 672, 4453, 1800, 3720
R_AP1, R_D1, R_AP2, R_D2 = 908, 4217, 2656, 3163
DD1 = 0.70
EXC = 8.0       # Dattorro's default is 16; half of it, a voice should not warble
MOD_HZ = 1.0
TAPS_L = [("dR1", 266, 1), ("dR1", 2974, 1), ("apR2", 1913, -1), ("dR2", 1996, 1),
          ("dL1", 1990, -1), ("apL2", 187, -1), ("dL2", 1066, -1)]
TAPS_R = [("dL1", 353, 1), ("dL1", 3627, 1), ("apL2", 1228, -1), ("dL2", 2673, 1),
          ("dR1", 2111, -1), ("apR2", 335, -1), ("dR2", 121, -1)]


def scale(n, fs):
    return int(round(n * fs / SR0))


def loop_seconds(fs):
    return sum(scale(n, fs) for n in (L_AP1, L_D1, L_AP2, L_D2, R_AP1, R_D1, R_AP2, R_D2)) / fs


def decay_for(t60, fs):
    return 10 ** (-3 * loop_seconds(fs) / (4 * t60))


class Line:
    """Delay line: read(k) is the value written k + 1 samples ago... see `ago`."""

    def __init__(self, n):
        self.buf = np.zeros(n)
        self.pos = 0  # next write position

    def ago(self, k):
        """The value written k samples before the most recent write (k = 0: the latest)."""
        n = len(self.buf)
        return self.buf[(self.pos - 1 - k) % n]

    def frac(self, d):
        """Linear interpolation at d samples back from the next write (d >= 1)."""
        i = int(np.floor(d))
        f = d - i
        return (1 - f) * self.ago(i - 1) + f * self.ago(i)

    def write(self, v):
        self.buf[self.pos] = v
        self.pos = (self.pos + 1) % len(self.buf)


def allpass(line, m, g, x):
    """Schroeder allpass with delay m: w = x + g*z, y = z - g*w, z = w delayed m."""
    z = line.ago(m - 1)
    w = x + g * z
    line.write(w)
    return z - g * w


def allpass_mod(line, d, g, x):
    z = line.frac(d)
    w = x + g * z
    line.write(w)
    return z - g * w


def reverb(x, fs, t60, pre_ms, lowcut, damp_hz):
    pre = int(round(pre_ms * 1e-3 * fs))
    xp = np.concatenate([np.zeros(pre), x])[: len(x)]
    if lowcut > 0:
        wc = 2 * fs * np.tan(np.pi * lowcut / fs)
        b, a = signal.bilinear([1, 0, 0], [1, wc * np.sqrt(2), wc * wc], fs)
        xp = signal.lfilter(b, a, xp)
    decay = decay_for(t60, fs)
    dd2 = min(0.5, max(0.25, decay + 0.15))
    damp = np.exp(-2 * np.pi * damp_hz / fs)
    exc = EXC * fs / SR0
    m = {k: scale(v, fs) for k, v in dict(apL1=L_AP1, dL1=L_D1, apL2=L_AP2, dL2=L_D2,
                                           apR1=R_AP1, dR1=R_D1, apR2=R_AP2, dR2=R_D2).items()}
    ins = [(Line(scale(n, fs)), scale(n, fs), g) for n, g in IN_AP]
    ln = {k: Line(v + (int(np.ceil(exc)) + 2 if k in ("apL1", "apR1") else 0)) for k, v in m.items()}
    taps_l = [(k, scale(t, fs), s) for k, t, s in TAPS_L]
    taps_r = [(k, scale(t, fs), s) for k, t, s in TAPS_R]
    w = 2 * np.pi * MOD_HZ / fs
    dl = dr = 0.0
    yl = np.empty(len(x))
    yr = np.empty(len(x))
    for n, v in enumerate(xp):
        for line, mm, g in ins:
            v = allpass(line, mm, g, v)
        fb_l = ln["dR2"].ago(m["dR2"] - 1)
        fb_r = ln["dL2"].ago(m["dL2"] - 1)
        # left half
        a = allpass_mod(ln["apL1"], m["apL1"] + exc * np.sin(w * n), -DD1, v + decay * fb_l)
        b = ln["dL1"].ago(m["dL1"] - 1)
        ln["dL1"].write(a)
        dl = (1 - damp) * b + damp * dl
        c = allpass(ln["apL2"], m["apL2"], dd2, decay * dl)
        ln["dL2"].write(c)
        # right half
        a = allpass_mod(ln["apR1"], m["apR1"] + exc * np.cos(w * n), -DD1, v + decay * fb_r)
        b = ln["dR1"].ago(m["dR1"] - 1)
        ln["dR1"].write(a)
        dr = (1 - damp) * b + damp * dr
        c = allpass(ln["apR2"], m["apR2"], dd2, decay * dr)
        ln["dR2"].write(c)
        yl[n] = 0.6 * sum(s * ln[k].ago(t) for k, t, s in taps_l)
        yr[n] = 0.6 * sum(s * ln[k].ago(t) for k, t, s in taps_r)
    return yl, yr


def main():
    src, dst = sys.argv[1], sys.argv[2]
    t60, pre, lowcut, damp, mix = (float(v) for v in sys.argv[3:8])
    stereo = len(sys.argv) > 8 and sys.argv[8] == "stereo"
    x, fs = sf.read(src, dtype="float64")
    if x.ndim > 1:
        x = x[:, 0]
    yl, yr = reverb(x, fs, t60, pre, lowcut, damp)
    out = np.stack([x + mix * yl, x + mix * yr], 1) if stereo else x + mix * (yl + yr) / 2
    sf.write(dst, out.astype(np.float32), fs, subtype="FLOAT")


if __name__ == "__main__":
    main()
