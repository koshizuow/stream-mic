"""Reference lookahead peak limiter (Signalsmith 2022 "Designing a straightforward limiter", same idea as
Hamalainen DAFx-02: a running minimum of the needed gain, then a finite smoothing that never goes above it).

  gmax[n] = min(1, T / |x[n]|)                       gain that keeps this sample at the threshold
  m[n]    = min(gmax[n-L-H+1 .. n])                  running minimum over lookahead L plus hold H
  r[n]    = min(m[n], r[n-1] + (m[n]-r[n-1]) * k)    exponential release (only moving up is slowed)
  s[n]    = two cascaded box filters, total support L
  y[n]    = x[n-L+1] * s[n]                          the signal is delayed by L-1 samples

Because every box output is an average of values each at or below gmax[n-L+1], the delayed sample never exceeds
the threshold: no overshoot by construction, unlike an attack filter (Hamalainen section 3.1).

With tp=True the detector looks at the signal upsampled 4x (true peak, ITU-R BS.1770 style), so peaks between
samples are caught too. Offline the upsampling is zero phase; a real-time version adds the interpolation filter's
delay to the lookahead.

With tp="causal" it uses the real-time version's interpolation filter (causal, TP_DELAY samples more delay)
instead, exactly as crates/dsp/src/limiter.rs does.

  python reflim.py IN OUT THRESH_DB LOOKAHEAD_MS HOLD_MS RELEASE_MS [tp|causal]
"""
import sys

import numpy as np
import soundfile as sf
from scipy.ndimage import minimum_filter1d
from scipy.signal import lfilter, resample_poly


def peak_env(x, tp):
    if not tp:
        return np.abs(x)
    up = resample_poly(x, 4, 1)
    # each input sample n covers upsampled points 4n-2 .. 4n+1 (around and between its neighbours)
    up = np.concatenate([np.zeros(2), up, np.zeros(4)])[: 4 * len(x) + 4]
    return np.abs(up[: 4 * len(x)].reshape(-1, 4)).max(1)


TP_DELAY = 8  # interpolation filter: 16 taps per phase, Kaiser (beta 8) windowed sinc


def tp_coef(p, j):
    """Coefficient for x[k + j] when interpolating at k + p/4 (same formula as crates/dsp limiter.rs)."""
    tau = p / 4 - j
    if abs(tau) >= TP_DELAY:
        return 0.0
    return np.sinc(tau) * np.i0(8.0 * np.sqrt(1 - (tau / TP_DELAY) ** 2)) / np.i0(8.0)


def peak_env_causal(x):
    """What the real-time limiter sees: e[n] is the true peak of sample n - TP_DELAY and the three
    interpolated points after it (zeros before the start)."""
    xp = np.concatenate([np.zeros(2 * TP_DELAY), x])
    k0 = np.arange(len(x)) - TP_DELAY  # sample index described at time n
    e = np.abs(xp[k0 + 2 * TP_DELAY])
    for p in (1, 2, 3):
        acc = np.zeros(len(x))
        for j in range(-(TP_DELAY - 1), TP_DELAY + 1):
            acc += tp_coef(p, j) * xp[k0 + j + 2 * TP_DELAY]
        e = np.maximum(e, np.abs(acc))
    return e


def limit(x, fs, thr_db, look_ms, hold_ms, rel_ms, tp=False):
    thr = 10 ** (thr_db / 20)
    L = max(1, int(round(look_ms * 1e-3 * fs)))
    H = int(round(hold_ms * 1e-3 * fs))
    env = peak_env_causal(x) if tp == "causal" else peak_env(x, tp)
    gmax = np.minimum(1.0, thr / np.maximum(env, 1e-12))
    # running minimum over the last L+H samples: a centred filter on a padded copy, shifted to look backward
    w = L + H
    c = minimum_filter1d(np.concatenate([np.ones(w), gmax]), size=w, mode="nearest")
    sh = w - (w - 1 - w // 2)
    m = c[sh: sh + len(gmax)]
    k = 1 - np.exp(-1 / max(1e-9, rel_ms * 1e-3 * fs))
    r = np.empty_like(m)
    prev = 1.0
    for i, v in enumerate(m):
        prev = v if v < prev else prev + (v - prev) * k
        r[i] = prev
    l1 = (L + 1) // 2
    l2 = L - l1 + 1
    s = lfilter(np.ones(l1) / l1, [1], np.concatenate([np.ones(L), r]))
    s = lfilter(np.ones(l2) / l2, [1], s)[L:]
    d = L - 1 + (TP_DELAY if tp == "causal" else 0)
    xd = np.concatenate([np.zeros(d), x])[: len(x)]
    return xd * s, s, d


def main():
    src, dst = sys.argv[1], sys.argv[2]
    thr, look, hold, rel = map(float, sys.argv[3:7])
    tp = sys.argv[7] if len(sys.argv) > 7 else False
    tp = True if tp == "tp" else tp
    x, fs = sf.read(src, dtype="float64")
    if x.ndim > 1:
        x = x[:, 0]
    y, _, _ = limit(x, fs, thr, look, hold, rel, tp)
    sf.write(dst, y.astype(np.float32), fs, subtype="FLOAT")


if __name__ == "__main__":
    main()
