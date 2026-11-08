"""Reference FDN reverb after Geraint Luff (Signalsmith), "Let's Write A Reverb" (2021), written from the article
(design/dsp/reverb.md), with Jot's per-line decay.

  x -> pre-delay -> low cut -> copy to N = 8 channels
    -> diffuser: 4 steps, each: per-channel delay (spread over the step's range), Hadamard, flip some signs
    -> feedback loop: per-channel delay D_c = SIZE * 2^(c/N), damping low pass, gain 10^(-3 D_c / (T60 fs)),
       Householder mix, plus the diffused input
    -> output where the loop is written (so the wet starts after the diffuser), even channels to the left,
       odd to the right

Every delay length comes from fixed fractions (no random numbers), so crates/dsp/src/reverb.rs can match it.

Usage: uv run python reffdn.py IN OUT.wav T60 PRE_MS LOWCUT_HZ DAMP_HZ MIX [stereo]
"""
import sys

import numpy as np
import soundfile as sf
from scipy import signal

N = 8
STEPS_MS = [4.0, 8.0, 16.0, 32.0]  # each step doubles; dense from the start (echo density 0.94), wet centred about 29 ms in
SIZE_MS = 45.0
FRACS = [0.33, 0.71, 0.12, 0.58, 0.91, 0.27, 0.44, 0.86]
FLIPS = [[1, -1, 1, 1, -1, 1, -1, -1], [-1, 1, 1, -1, 1, -1, 1, 1],
         [1, 1, -1, 1, -1, -1, 1, -1], [-1, -1, 1, -1, 1, 1, -1, 1]]
OUT_GAIN = 1.0


def hadamard(v):
    v = v.copy()
    h = 1
    while h < len(v):
        for i in range(0, len(v), 2 * h):
            a = v[i:i + h].copy()
            b = v[i + h:i + 2 * h].copy()
            v[i:i + h] = a + b
            v[i + h:i + 2 * h] = a - b
        h *= 2
    return v / np.sqrt(len(v))


def diffuser_delays(fs):
    out = []
    for s, r in enumerate(STEPS_MS):
        rng = r * 1e-3 * fs
        out.append([int(round(rng * (c + FRACS[(c + 3 * s) % N]) / N)) + 1 for c in range(N)])
    return out


def loop_delays(fs):
    return [int(round(SIZE_MS * 1e-3 * fs * 2 ** (c / N))) for c in range(N)]


def reverb(x, fs, t60, pre_ms, lowcut, damp_hz):
    pre = int(round(pre_ms * 1e-3 * fs))
    xp = np.concatenate([np.zeros(pre), x])[: len(x)]
    if lowcut > 0:
        wc = 2 * fs * np.tan(np.pi * lowcut / fs)
        b, a = signal.bilinear([1, 0, 0], [1, wc * np.sqrt(2), wc * wc], fs)
        xp = signal.lfilter(b, a, xp)
    dd = diffuser_delays(fs)
    ld = loop_delays(fs)
    dbuf = [np.zeros((N, max(d))) for d in dd]
    dpos = [0] * len(dd)
    lbuf = np.zeros((N, max(ld)))
    lpos = 0
    lens = np.array(ld)
    gains = 10 ** (-3 * lens / (t60 * fs))
    damp = np.exp(-2 * np.pi * damp_hz / fs)
    lp = np.zeros(N)
    yl = np.empty(len(x))
    yr = np.empty(len(x))
    ch = np.arange(N)
    for n, v in enumerate(xp):
        u = np.full(N, v)
        for s, d in enumerate(dd):
            buf = dbuf[s]
            m = buf.shape[1]
            p = dpos[s]
            out = buf[ch, (p - np.array(d)) % m]
            buf[:, p] = u
            dpos[s] = (p + 1) % m
            u = hadamard(out) * FLIPS[s]
        m = lbuf.shape[1]
        out = lbuf[ch, (lpos - lens) % m]
        lp = (1 - damp) * out + damp * lp
        fb = lp * gains
        fb = fb - (2.0 / N) * fb.sum()
        w = fb + u
        lbuf[:, lpos] = w
        lpos = (lpos + 1) % m
        # taken where the loop is written, so the wet starts after the diffuser (a few ms), not after a loop delay
        yl[n] = OUT_GAIN * w[0::2].sum() / np.sqrt(N / 2)
        yr[n] = OUT_GAIN * w[1::2].sum() / np.sqrt(N / 2)
    return yl, yr


MUSIC_ON_DB = -50.0
MUSIC_TAU_S = 0.3


def follow_mix(music, n, fs, mix, mix_dry, follow_s):
    """Per-sample wet amount: glides to mix while the music's 300 ms power is over MUSIC_ON_DB, else to mix_dry."""
    a = np.exp(-1 / (MUSIC_TAU_S * fs))
    rate = abs(mix - mix_dry) / (max(follow_s, 0.01) * fs)
    p = 0.0
    cur = mix_dry
    out = np.empty(n)
    for i in range(n):
        m = music[i] if i < len(music) else 0.0
        p = a * p + (1 - a) * m * m
        target = mix if 10 * np.log10(p + 1e-30) > MUSIC_ON_DB else mix_dry
        cur = min(cur + rate, target) if cur < target else max(cur - rate, target)
        out[i] = cur
    return out


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
